//! Native desktop OS adapters. No WebView is used to render the application.
use gamestrike::{
    application::{DesktopHost, LaunchOutcome, RunningGame},
    stores::{Launch, Registry, ScanEnv, StoreLibrary},
};
use std::{
    io::Read,
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[cfg(windows)]
mod windows;

pub struct SystemHost;

impl DesktopHost for SystemHost {
    fn refresh_stores(&mut self, stores: &StoreLibrary) -> Result<(), String> {
        let registry = registry();
        stores
            .refresh(&ScanEnv::for_this_computer(registry.as_ref()))
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn launch_store(&mut self, target: &Launch) -> Result<LaunchOutcome, String> {
        gamestrike::stores::check_launch(target).map_err(|error| error.to_string())?;
        match target {
            Launch::Uri { uri } => open_uri(uri),
            Launch::Program {
                path,
                args,
                working_dir,
            } => {
                if !path.is_file()
                    && !(cfg!(target_os = "macos")
                        && path.is_dir()
                        && path.extension().is_some_and(|ext| ext == "app"))
                {
                    return Err("The selected program is missing".into());
                }
                #[cfg(windows)]
                if path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| {
                        ext.eq_ignore_ascii_case("lnk") || ext.eq_ignore_ascii_case("url")
                    })
                {
                    windows::open(path.as_os_str())?;
                    return Ok(LaunchOutcome::HandedOff);
                }
                #[cfg(target_os = "linux")]
                if path.extension().is_some_and(|ext| ext == "desktop") {
                    return spawn_handoff(Command::new("gio").arg("launch").arg(path));
                }
                #[cfg(target_os = "macos")]
                if path.is_dir() && path.extension().is_some_and(|ext| ext == "app") {
                    let mut command = Command::new("open");
                    command.arg("-a").arg(path);
                    if !args.is_empty() {
                        command.arg("--args").args(args);
                    }
                    return spawn_handoff(&mut command);
                }
                let mut command = Command::new(path);
                command.args(args);
                if let Some(folder) = working_dir.as_deref().or_else(|| path.parent()) {
                    command.current_dir(folder);
                }
                spawn_game(&mut command)
            }
        }
    }

    fn launch_html(&mut self, root: &Path, id: Uuid) -> Result<LaunchOutcome, String> {
        let player = gamestrike::sibling_player().map_err(|error| error.to_string())?;
        if !player.is_file() {
            return Err("The optional isolated HTML player is missing beside GameStrike".into());
        }
        let mut command = Command::new(player);
        command
            .arg("--library")
            .arg(root)
            .arg("--id")
            .arg(id.to_string())
            .arg("--offline");
        spawn_game(&mut command)
    }
}

fn registry() -> Box<dyn Registry> {
    #[cfg(windows)]
    {
        Box::new(windows::WindowsRegistry)
    }
    #[cfg(not(windows))]
    {
        Box::new(gamestrike::stores::NoRegistry)
    }
}

fn open_uri(uri: &str) -> Result<LaunchOutcome, String> {
    #[cfg(windows)]
    {
        windows::open(std::ffi::OsStr::new(uri))?;
        Ok(LaunchOutcome::HandedOff)
    }
    #[cfg(target_os = "linux")]
    {
        spawn_handoff(Command::new("xdg-open").arg(uri))
    }
    #[cfg(target_os = "macos")]
    {
        spawn_handoff(Command::new("open").arg(uri))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn spawn_handoff(command: &mut Command) -> Result<LaunchOutcome, String> {
    spawn_process(command).map(|process| LaunchOutcome::HandoffProcess(Box::new(process)))
}

fn spawn_game(command: &mut Command) -> Result<LaunchOutcome, String> {
    spawn_process(command).map(|process| LaunchOutcome::Tracked(Box::new(process)))
}

fn spawn_process(command: &mut Command) -> Result<Process, String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not start the game: {error}"))?;
    let error = Arc::new(Mutex::new(None));
    let mut readers = Vec::new();
    for pipe in [
        child
            .stdout
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>),
        child
            .stderr
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let error = Arc::clone(&error);
        readers.push(thread::spawn(move || drain_errors(pipe, error)));
    }
    Ok(Process {
        child,
        error,
        readers,
        exited: None,
        drain_started: None,
    })
}

struct Process {
    child: Child,
    error: Arc<Mutex<Option<String>>>,
    readers: Vec<thread::JoinHandle<()>>,
    exited: Option<ExitStatus>,
    drain_started: Option<Instant>,
}

impl RunningGame for Process {
    fn poll_exit(&mut self) -> Option<Result<(), String>> {
        if self.exited.is_none() {
            match self.child.try_wait() {
                Ok(None) => return None,
                Ok(Some(status)) => {
                    self.exited = Some(status);
                    self.drain_started = Some(Instant::now());
                }
                Err(error) => {
                    return Some(Err(format!("Could not observe the game's exit: {error}")));
                }
            }
        }
        if !self.readers.iter().all(|reader| reader.is_finished())
            && self
                .drain_started
                .is_some_and(|start| start.elapsed() < Duration::from_millis(800))
        {
            return None;
        }
        self.exited.as_ref().map(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(self
                    .error
                    .lock()
                    .ok()
                    .and_then(|error| error.clone())
                    .unwrap_or_else(|| format!("The game exited with {status}")))
            }
        })
    }
}

// Keep draining inherited pipes without unbounded line allocations or blocking native rendering.
fn drain_errors(mut pipe: Box<dyn Read + Send>, error: Arc<Mutex<Option<String>>>) {
    let mut buffer = [0_u8; 4096];
    let mut line = Vec::with_capacity(1024);
    while let Ok(count) = pipe.read(&mut buffer) {
        if count == 0 {
            break;
        }
        for &byte in &buffer[..count] {
            if byte == b'\n' {
                record_error(&line, &error);
                line.clear();
            } else if line.len() < 1024 {
                line.push(byte);
            }
        }
    }
    record_error(&line, &error);
}

fn record_error(line: &[u8], error: &Mutex<Option<String>>) {
    let text = String::from_utf8_lossy(line);
    if let Some(detail) = text.strip_prefix("Error: ") {
        if let Ok(mut error) = error.lock() {
            *error = Some(detail.chars().take(240).collect());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn final_unterminated_player_error_is_not_lost() {
        let error = Arc::new(Mutex::new(None));
        drain_errors(
            Box::new(std::io::Cursor::new(b"log\nError: explicit failure")),
            Arc::clone(&error),
        );
        assert_eq!(error.lock().unwrap().as_deref(), Some("explicit failure"));
    }
}
