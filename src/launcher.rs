//! Trusted local library UI. Imported HTML is never loaded in this WebView.
#[cfg(not(windows))]
fn main() {
    eprintln!("The GameStrike visual library currently requires Windows.");
}

#[cfg(windows)]
mod windows_launcher {
    use gamestrike::{ImportOutcome, Library};
    use serde::Deserialize;
    use serde_json::{Value, json};
    use std::{
        borrow::Cow,
        env,
        error::Error,
        ffi::OsString,
        fs,
        io::{BufRead, BufReader},
        mem::size_of,
        os::windows::ffi::OsStringExt,
        path::{Path, PathBuf},
        process::{Child, Command, ExitStatus, Stdio},
        sync::{Arc, Mutex},
        thread,
        time::Duration,
    };
    use tao::{
        dpi::LogicalSize,
        event::{Event, WindowEvent},
        event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
        platform::windows::WindowExtWindows,
        window::WindowBuilder,
    };
    use uuid::Uuid;
    use webview2_com::{
        CapturePreviewCompletedHandler,
        Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
    };
    use windows::Win32::System::Com::{STREAM_SEEK_SET, StructuredStorage::CreateStreamOnHGlobal};
    use windows_sys::Win32::UI::Controls::Dialogs::{
        CommDlgExtendedError, GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR,
        OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };
    use wry::{
        WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows,
        http::{Request, Response, StatusCode, header},
    };

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
    enum CommandMessage {
        Ready {},
        PickAndImport {},
        LaunchGame { id: String, network: bool },
        RenameGame { id: String, title: String },
        ResetSaves { id: String },
        RemoveGame { id: String, keep_saves: bool },
    }

    enum UiEvent {
        Command(CommandMessage),
        CapturePreview,
        CaptureComplete(std::result::Result<(), String>),
        ImportResult(std::result::Result<ImportOutcome, String>),
        PlayerExited {
            id: Uuid,
            network: bool,
            report: PlayerReport,
        },
    }

    /// What the launcher learned from a finished player process.
    #[derive(Debug, Default)]
    struct PlayerReport {
        status: Option<ExitStatus>,
        wait_error: Option<String>,
        blocked: Option<usize>,
        error_line: Option<String>,
    }

    #[derive(Clone, Copy)]
    enum CaptureView {
        Library,
        LongTitle,
        Launch,
        Manage,
        About,
    }

    pub fn run() -> Result<(), Box<dyn Error>> {
        let (root, mut capture_path, window_size, capture_view) = launch_options()?;
        let library = Arc::new(Library::open(&root)?);
        let root = fs::canonicalize(root)?;
        let event_loop = EventLoopBuilder::<UiEvent>::with_user_event().build();
        let window = WindowBuilder::new()
            .with_title("GameStrike")
            .with_inner_size(window_size)
            .with_min_inner_size(LogicalSize::new(880.0, 620.0))
            .build(&event_loop)?;
        let proxy = event_loop.create_proxy();
        let background_proxy = event_loop.create_proxy();
        let webview = WebViewBuilder::new()
            .with_custom_protocol("gamestrikeui".into(), |_, request| serve_ui(request))
            .with_https_scheme(true)
            .with_url("gamestrikeui://localhost/index.html")
            .with_navigation_handler(|url| url == "about:blank" || is_ui_origin(&url))
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .with_download_started_handler(|_, _| false)
            .with_drag_drop_handler(|_| true)
            .with_default_context_menus(false)
            .with_devtools(false)
            .with_ipc_handler(move |request| {
                if let Some(command) = parse_command(&request.uri().to_string(), request.body()) {
                    let _ = proxy.send_event(UiEvent::Command(command));
                }
            })
            .build(&window)?;

        let mut view_prepared = false;
        let mut running: Option<Uuid> = None;
        let mut picking = false;
        event_loop.run(move |event, _, flow| {
            let _keep_alive = (&window, &webview);
            *flow = ControlFlow::Wait;
            match event {
                Event::UserEvent(UiEvent::Command(CommandMessage::Ready {})) => {
                    send_library(&webview, &library, running);
                    if capture_path.is_some() {
                        let proxy = background_proxy.clone();
                        thread::spawn(move || {
                            thread::sleep(Duration::from_millis(1600));
                            let _ = proxy.send_event(UiEvent::CapturePreview);
                        });
                    }
                }
                Event::UserEvent(UiEvent::CapturePreview) => {
                    let prepare = if view_prepared {
                        None
                    } else {
                        match capture_view {
                            CaptureView::Library => None,
                            CaptureView::LongTitle => Some("window.GameStrike.capture('long')"),
                            CaptureView::Launch => Some("window.GameStrike.capture('launch')"),
                            CaptureView::Manage => Some("window.GameStrike.capture('manage')"),
                            CaptureView::About => Some("window.GameStrike.capture('about')"),
                        }
                    };
                    if let Some(script) = prepare {
                        view_prepared = true;
                        if let Err(error) = webview.evaluate_script(script) {
                            eprintln!("Could not prepare launcher preview: {error}");
                            std::process::exit(2);
                        }
                        let proxy = background_proxy.clone();
                        thread::spawn(move || {
                            thread::sleep(Duration::from_millis(450));
                            let _ = proxy.send_event(UiEvent::CapturePreview);
                        });
                    } else if let Some(path) = capture_path.take()
                        && let Err(error) = capture_preview(&webview, path, background_proxy.clone())
                    {
                        eprintln!("Could not capture launcher: {error}");
                        std::process::exit(2);
                    }
                }
                Event::UserEvent(UiEvent::CaptureComplete(result)) => {
                    match result {
                        Ok(()) => println!("Launcher preview captured."),
                        Err(error) => {
                            eprintln!("Could not capture launcher: {error}");
                            std::process::exit(2);
                        }
                    }
                    *flow = ControlFlow::Exit;
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::PickAndImport {})) => {
                    // The picker is modal; a queued second request while it is open is ignored.
                    if picking {
                        return;
                    }
                    picking = true;
                    let picked = pick_game_file(window.hwnd());
                    match picked {
                        Ok(Some(path)) => {
                            let name = path
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "game".to_owned());
                            send(&webview, json!({"type":"importing", "name": name}));
                            let library = Arc::clone(&library);
                            let proxy = background_proxy.clone();
                            thread::spawn(move || {
                                let result = library.import_with_result(path).map_err(|error| error.to_string());
                                let _ = proxy.send_event(UiEvent::ImportResult(result));
                            });
                        }
                        Ok(None) => {
                            picking = false;
                            send(&webview, json!({"type":"pick_cancelled"}));
                        }
                        Err(error) => {
                            picking = false;
                            send(&webview, json!({"type":"error", "operation":"import", "message":error}));
                        }
                    }
                }
                Event::UserEvent(UiEvent::ImportResult(result)) => {
                    picking = false;
                    match result {
                        Ok(outcome) => send(&webview, json!({
                            "type":"imported",
                            "game":outcome.game,
                            "already_present":outcome.already_present,
                            "restored_saves":outcome.restored_saves,
                        })),
                        Err(error) => send(
                            &webview,
                            json!({"type":"error", "operation":"import", "message":format!("Import rejected: {error}")}),
                        ),
                    }
                    send_library(&webview, &library, running);
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::LaunchGame { id, network })) => {
                    if running.is_some() {
                        send(&webview, json!({"type":"error", "operation":"launch", "id":id, "message":"A game is already open. Close its window first."}));
                        return;
                    }
                    match launch(&library, &root, &id, network) {
                        Ok((uuid, child)) => {
                            running = Some(uuid);
                            let _ = library.record_play(uuid);
                            send(&webview, json!({"type":"player_started", "id":uuid, "network":network}));
                            send_library(&webview, &library, running);
                            watch_player(child, uuid, network, background_proxy.clone());
                        }
                        Err(error) => send(&webview, json!({"type":"error", "operation":"launch", "id":id, "message":error})),
                    }
                }
                Event::UserEvent(UiEvent::PlayerExited { id, network, report }) => {
                    if running == Some(id) {
                        running = None;
                    }
                    let (message, ok) = describe_exit(&report, network);
                    send(&webview, json!({"type":"player_exited", "id":id, "message":message, "ok":ok}));
                    send_library(&webview, &library, running);
                    window.set_focus();
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::RenameGame { id, title })) => {
                    let result = parse_id(&id).and_then(|uuid| {
                        library.rename(uuid, &title).map_err(|error| error.to_string())
                    });
                    match result {
                        Ok(game) => send(&webview, json!({"type":"managed", "id":id, "message":format!("Renamed to {}.", game.title)})),
                        Err(error) => send(&webview, json!({"type":"error", "operation":"manage", "id":id, "message":format!("Could not rename: {error}")})),
                    }
                    send_library(&webview, &library, running);
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::ResetSaves { id })) => {
                    let result = parse_id(&id).and_then(|uuid| {
                        if running == Some(uuid) {
                            return Err("Close the game before clearing its saves.".to_owned());
                        }
                        library.reset_saves(uuid).map_err(|error| error.to_string())
                    });
                    match result {
                        Ok(()) => send(&webview, json!({"type":"managed", "id":id, "message":"Saves cleared. The next launch starts fresh."})),
                        Err(error) => send(&webview, json!({"type":"error", "operation":"manage", "id":id, "message":format!("Could not clear saves: {error}")})),
                    }
                    send_library(&webview, &library, running);
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::RemoveGame { id, keep_saves })) => {
                    let result = parse_id(&id).and_then(|uuid| {
                        if running == Some(uuid) {
                            return Err("Close the game before removing it.".to_owned());
                        }
                        library.remove(uuid, keep_saves).map_err(|error| error.to_string())
                    });
                    match result {
                        Ok(()) => send(&webview, json!({"type":"removed", "id":id, "message": if keep_saves {
                            "Game removed. Its saves return if you add the same file again."
                        } else {
                            "Game and saves removed."
                        }})),
                        Err(error) => send(&webview, json!({"type":"error", "operation":"manage", "id":id, "message":format!("Could not remove: {error}")})),
                    }
                    send_library(&webview, &library, running);
                }
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                } => *flow = ControlFlow::Exit,
                _ => {}
            }
        })
    }

    /// Accepts only small JSON commands from the bundled UI origin.
    fn parse_command(origin: &str, body: &str) -> Option<CommandMessage> {
        if !is_ui_origin(origin) || body.len() > 1024 {
            return None;
        }
        serde_json::from_str(body).ok()
    }

    fn parse_id(raw: &str) -> std::result::Result<Uuid, String> {
        Uuid::parse_str(raw).map_err(|_| "Invalid game ID".to_owned())
    }

    fn watch_player(mut child: Child, id: Uuid, network: bool, proxy: EventLoopProxy<UiEvent>) {
        let blocked = Arc::new(Mutex::new(None));
        let error_line = Arc::new(Mutex::new(None));
        let readers: Vec<_> = [
            child
                .stdout
                .take()
                .map(|out| Box::new(out) as Box<dyn std::io::Read + Send>),
            child
                .stderr
                .take()
                .map(|err| Box::new(err) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .flatten()
        .map(|stream| {
            let blocked = Arc::clone(&blocked);
            let error_line = Arc::clone(&error_line);
            thread::spawn(move || {
                for line in BufReader::new(stream)
                    .lines()
                    .map_while(Result::ok)
                    .take(10_000)
                {
                    if let Some(count) = line.strip_prefix(BLOCKED_PREFIX) {
                        *blocked.lock().unwrap() = count.trim().parse().ok();
                    } else if let Some(error) = player_error(&line) {
                        *error_line.lock().unwrap() = Some(error);
                    }
                }
            })
        })
        .collect();
        thread::spawn(move || {
            let mut report = PlayerReport::default();
            match child.wait() {
                Ok(status) => report.status = Some(status),
                Err(error) => report.wait_error = Some(error.to_string()),
            }
            // Pipes can stay open if the WebView runtime inherited them; wait briefly only.
            let deadline = std::time::Instant::now() + Duration::from_millis(800);
            for reader in readers {
                while !reader.is_finished() && std::time::Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(20));
                }
            }
            report.blocked = *blocked.lock().unwrap();
            report.error_line = error_line.lock().unwrap().clone();
            let _ = proxy.send_event(UiEvent::PlayerExited {
                id,
                network,
                report,
            });
        });
    }

    /// Extracts the message from the player's `Error: ...` exit line.
    fn player_error(line: &str) -> Option<String> {
        let rest = line.strip_prefix("Error: ")?;
        let message = rest
            .split_once("error: \"")
            .and_then(|(_, tail)| tail.split_once('"'))
            .map(|(message, _)| message.to_owned())
            .unwrap_or_else(|| rest.trim_matches('"').to_owned());
        Some(message.chars().take(240).collect())
    }

    fn describe_exit(report: &PlayerReport, network: bool) -> (String, bool) {
        match (&report.status, &report.wait_error) {
            (Some(status), _) if status.success() => {
                let blocked = match (network, report.blocked) {
                    (false, Some(0)) => " Offline mode blocked no network requests.".to_owned(),
                    (false, Some(1)) => " Offline mode blocked 1 network request.".to_owned(),
                    (false, Some(count)) => {
                        format!(
                            " Offline mode blocked {count} network requests. If the game looked incomplete, try Play with network."
                        )
                    }
                    _ => String::new(),
                };
                (
                    format!("Game closed. Your progress is saved on this PC.{blocked}"),
                    true,
                )
            }
            (Some(status), _) => (
                match &report.error_line {
                    Some(error) => format!("The game could not open: {error}."),
                    None => format!("The player stopped unexpectedly ({status})."),
                },
                false,
            ),
            (None, Some(error)) => (format!("Could not track the player: {error}"), false),
            (None, None) => ("Could not track the player.".to_owned(), false),
        }
    }

    /// Library root, optional capture PNG, window size, and capture view.
    type LaunchOptions = (PathBuf, Option<PathBuf>, LogicalSize<f64>, CaptureView);

    fn launch_options() -> Result<LaunchOptions, Box<dyn Error>> {
        let args: Vec<_> = env::args_os().collect();
        let default_size = LogicalSize::new(1250.0, 800.0);
        match args.as_slice() {
            [_] => Ok((
                PathBuf::from(env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?)
                    .join("GameStrike")
                    .join("Library"),
                None,
                default_size,
                CaptureView::Library,
            )),
            [_, flag, root] if flag == "--library" => {
                Ok((PathBuf::from(root), None, default_size, CaptureView::Library))
            }
            [_, flag, root, capture_flag, output]
                if flag == "--library" && capture_flag == "--capture-preview" =>
            {
                Ok((PathBuf::from(root), Some(PathBuf::from(output)), default_size, CaptureView::Library))
            }
            [_, flag, root, capture_flag, output, view_flag, view]
                if flag == "--library"
                    && capture_flag == "--capture-preview"
                    && view_flag == "--capture-view" =>
            {
                Ok((PathBuf::from(root), Some(PathBuf::from(output)), default_size, parse_capture_view(view)?))
            }
            [_, flag, root, capture_flag, output, size_flag, width, height]
                if flag == "--library"
                    && capture_flag == "--capture-preview"
                    && size_flag == "--window-size" =>
            {
                let width = width.to_str().ok_or("window width is not UTF-8")?.parse::<u32>()?;
                let height = height.to_str().ok_or("window height is not UTF-8")?.parse::<u32>()?;
                if !(880..=3840).contains(&width) || !(620..=2160).contains(&height) {
                    return Err("capture window size must be between 880x620 and 3840x2160".into());
                }
                Ok((
                    PathBuf::from(root),
                    Some(PathBuf::from(output)),
                    LogicalSize::new(width as f64, height as f64),
                    CaptureView::Library,
                ))
            }
            _ => Err(
                "usage: gamestrike-launcher [--library DIRECTORY [--capture-preview PNG [--capture-view library|long|launch|manage|about | --window-size WIDTH HEIGHT]]]".into(),
            ),
        }
    }

    fn parse_capture_view(raw: &OsString) -> Result<CaptureView, Box<dyn Error>> {
        match raw.to_str() {
            Some("library") => Ok(CaptureView::Library),
            Some("long") => Ok(CaptureView::LongTitle),
            Some("launch") => Ok(CaptureView::Launch),
            Some("manage") => Ok(CaptureView::Manage),
            Some("about") => Ok(CaptureView::About),
            _ => Err("capture view must be library, long, launch, manage, or about".into()),
        }
    }

    fn capture_preview(
        webview: &wry::WebView,
        path: PathBuf,
        proxy: EventLoopProxy<UiEvent>,
    ) -> Result<(), Box<dyn Error>> {
        // WebView2 captures the rendered library, rather than a staged HTML mock.
        let stream = unsafe { CreateStreamOnHGlobal(Default::default(), true)? };
        let output = stream.clone();
        let handler = CapturePreviewCompletedHandler::create(Box::new(move |completed| {
            let result = (|| -> std::result::Result<(), String> {
                completed.map_err(|error| error.to_string())?;
                unsafe { output.Seek(0, STREAM_SEEK_SET, None) }
                    .map_err(|error| error.to_string())?;
                let mut png = Vec::new();
                let mut chunk = [0u8; 8192];
                loop {
                    let mut count = 0u32;
                    unsafe {
                        output.Read(
                            chunk.as_mut_ptr().cast(),
                            chunk.len() as u32,
                            Some(&mut count),
                        )
                    }
                    .ok()
                    .map_err(|error| error.to_string())?;
                    if count == 0 {
                        break;
                    }
                    png.extend_from_slice(&chunk[..count as usize]);
                    if png.len() > 16 * 1024 * 1024 {
                        return Err("launcher preview exceeds 16 MiB".to_owned());
                    }
                }
                if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
                    return Err("launcher preview is not a PNG".to_owned());
                }
                fs::write(path, png).map_err(|error| error.to_string())
            })();
            let _ = proxy.send_event(UiEvent::CaptureComplete(result));
            Ok(())
        }));
        unsafe {
            webview.webview().CapturePreview(
                COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                &stream,
                &handler,
            )?;
        }
        Ok(())
    }

    fn launch(
        library: &Library,
        root: &Path,
        raw_id: &str,
        network: bool,
    ) -> std::result::Result<(Uuid, Child), String> {
        let id = parse_id(raw_id)?;
        library
            .player_paths(id)
            .map_err(|error| error.to_string())?;
        let player = env::current_exe()
            .map_err(|error| error.to_string())?
            .with_file_name("gamestrike-player.exe");
        if !player.is_file() {
            return Err(
                "GameStrike player is missing. Build gamestrike-player beside the launcher.".into(),
            );
        }
        Command::new(player)
            .arg("--library")
            .arg(root)
            .arg("--id")
            .arg(id.to_string())
            .arg(if network {
                "--allow-network"
            } else {
                "--offline"
            })
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map(|child| (id, child))
            .map_err(|error| format!("Could not start player: {error}"))
    }

    const BLOCKED_PREFIX: &str = "gamestrike-network-blocked=";

    fn send_library(webview: &wry::WebView, library: &Library, running: Option<Uuid>) {
        match library.scan() {
            Ok(scan) => send(
                webview,
                json!({"type":"library", "games":scan.games, "problems":scan.problems, "running":running}),
            ),
            Err(error) => send(
                webview,
                json!({"type":"error", "operation":"library", "message":format!("Library could not be read: {error}")}),
            ),
        }
    }

    fn send(webview: &wry::WebView, payload: Value) {
        // JSON strings stay quoted inside the JS expression; titles are rendered with textContent.
        let script = format!("window.GameStrike.receive({payload});");
        if let Err(error) = webview.evaluate_script(&script) {
            eprintln!("Could not update GameStrike library: {error}");
        }
    }

    fn is_ui_origin(url: &str) -> bool {
        let Ok(uri) = url.parse::<wry::http::Uri>() else {
            return false;
        };
        (uri.scheme_str() == Some("https")
            && uri
                .authority()
                .is_some_and(|authority| authority.as_str() == "gamestrikeui.localhost"))
            || (uri.scheme_str() == Some("gamestrikeui")
                && uri
                    .authority()
                    .is_some_and(|authority| authority.as_str() == "localhost"))
    }

    fn serve_ui(request: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
        let (status, body, mime) = if request.method() != wry::http::Method::GET
            || !is_ui_origin(&request.uri().to_string())
        {
            (StatusCode::FORBIDDEN, &b"denied"[..], "text/plain")
        } else {
            match request.uri().path() {
                "/" | "/index.html" => (
                    StatusCode::OK,
                    &include_bytes!("../ui/index.html")[..],
                    "text/html; charset=utf-8",
                ),
                "/app.js" => (
                    StatusCode::OK,
                    &include_bytes!("../ui/app.js")[..],
                    "text/javascript; charset=utf-8",
                ),
                "/core.js" => (
                    StatusCode::OK,
                    &include_bytes!("../ui/core.js")[..],
                    "text/javascript; charset=utf-8",
                ),
                "/styles.css" => (
                    StatusCode::OK,
                    &include_bytes!("../ui/styles.css")[..],
                    "text/css; charset=utf-8",
                ),
                "/mark.svg" => (
                    StatusCode::OK,
                    &include_bytes!("../assets/branding/gamestrike-mark.svg")[..],
                    "image/svg+xml",
                ),
                "/pwds.svg" => (
                    StatusCode::OK,
                    &include_bytes!("../assets/branding/PWDS-logo-orange.svg")[..],
                    "image/svg+xml",
                ),
                _ => (StatusCode::NOT_FOUND, &b"not found"[..], "text/plain"),
            }
        };
        let mut response = Response::new(Cow::Borrowed(body));
        *response.status_mut() = status;
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, header::HeaderValue::from_static(mime));
        response.headers_mut().insert(header::HeaderName::from_static("content-security-policy"), header::HeaderValue::from_static("default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; base-uri 'none'; frame-src 'none'; form-action 'none'; connect-src 'none'"));
        response.headers_mut().insert(
            header::HeaderName::from_static("x-content-type-options"),
            header::HeaderValue::from_static("nosniff"),
        );
        response
    }

    fn pick_game_file(owner: isize) -> std::result::Result<Option<PathBuf>, String> {
        let mut selected = vec![0u16; 32_768];
        let filter: Vec<u16> = "HTML games (*.html;*.htm;*.zip)\0*.html;*.htm;*.zip\0HTML files (*.html;*.htm)\0*.html;*.htm\0ZIP archives (*.zip)\0*.zip\0\0".encode_utf16().collect();
        let title: Vec<u16> = "Add a local HTML game\0".encode_utf16().collect();
        let mut dialog = OPENFILENAMEW {
            lStructSize: size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: owner as *mut _,
            lpstrFilter: filter.as_ptr(),
            lpstrFile: selected.as_mut_ptr(),
            nMaxFile: selected.len() as u32,
            lpstrTitle: title.as_ptr(),
            Flags: OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
            ..Default::default()
        };
        // The dialog supplies only a selected path; Library::import validates the file and ZIP.
        if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
            let code = unsafe { CommDlgExtendedError() };
            return if code == 0 {
                Ok(None)
            } else {
                Err(format!("File picker failed (Windows error {code:#x})."))
            };
        }
        let end = selected
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(selected.len());
        Ok(Some(PathBuf::from(OsString::from_wide(&selected[..end]))))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn ui_protocol_serves_only_bundled_paths_on_its_exact_origin() {
            let allowed = Request::builder()
                .uri("https://gamestrikeui.localhost/index.html")
                .body(Vec::new())
                .unwrap();
            let response = serve_ui(allowed);
            assert_eq!(response.status(), StatusCode::OK);
            assert!(response.body().as_ref().starts_with(b"<!doctype html>"));

            let mark = Request::builder()
                .uri("https://gamestrikeui.localhost/mark.svg")
                .body(Vec::new())
                .unwrap();
            let mark_response = serve_ui(mark);
            assert_eq!(mark_response.status(), StatusCode::OK);
            assert_eq!(
                mark_response
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .unwrap()
                    .as_bytes(),
                b"image/svg+xml"
            );
            assert!(mark_response.body().as_ref().starts_with(b"<svg"));

            let csp = mark_response
                .headers()
                .get("content-security-policy")
                .unwrap()
                .to_str()
                .unwrap();
            assert!(csp.contains("default-src 'none'") && csp.contains("connect-src 'none'"));

            for url in [
                "https://other.localhost/index.html",
                "https://gamestrikeui.localhost/fox.png",
                "https://gamestrikeui.localhost/../Cargo.toml",
                "https://gamestrike1234.localhost/index.html",
                "https://gamestrikeui.localhost.evil/index.html",
                "https://gamestrikeui.localhost/private.txt",
            ] {
                let request = Request::builder().uri(url).body(Vec::new()).unwrap();
                assert_ne!(serve_ui(request).status(), StatusCode::OK, "{url}");
            }
        }

        #[test]
        fn ipc_accepts_only_known_commands_from_the_ui_origin() {
            let ui = "https://gamestrikeui.localhost/index.html";
            let id = Uuid::new_v4().to_string();
            assert_eq!(
                parse_command(
                    ui,
                    &format!(r#"{{"type":"launch_game","id":"{id}","network":false}}"#)
                ),
                Some(CommandMessage::LaunchGame {
                    id: id.clone(),
                    network: false
                })
            );
            assert_eq!(
                parse_command(
                    ui,
                    &format!(r#"{{"type":"remove_game","id":"{id}","keep_saves":true}}"#)
                ),
                Some(CommandMessage::RemoveGame {
                    id: id.clone(),
                    keep_saves: true
                })
            );
            for (origin, body) in [
                // Game origins and look-alikes never reach the launcher.
                (
                    format!(
                        "https://gamestrike{}.localhost/index.html",
                        Uuid::new_v4().simple()
                    ),
                    r#"{"type":"ready"}"#.to_owned(),
                ),
                (
                    "https://gamestrikeui.localhost.evil/".to_owned(),
                    r#"{"type":"ready"}"#.to_owned(),
                ),
                (
                    "file:///C:/index.html".to_owned(),
                    r#"{"type":"ready"}"#.to_owned(),
                ),
                // The UI never supplies a file path, and network must be explicit.
                (
                    ui.to_owned(),
                    r#"{"type":"pick_and_import","path":"C:\\x.zip"}"#.to_owned(),
                ),
                (
                    ui.to_owned(),
                    format!(r#"{{"type":"launch_game","id":"{id}"}}"#),
                ),
                (
                    ui.to_owned(),
                    r#"{"type":"run_shell","command":"calc"}"#.to_owned(),
                ),
                (
                    ui.to_owned(),
                    format!(
                        r#"{{"type":"rename_game","id":"{id}","title":"{}"}}"#,
                        "x".repeat(1100)
                    ),
                ),
                (ui.to_owned(), "not json".to_owned()),
            ] {
                assert_eq!(parse_command(&origin, &body), None, "{origin} {body}");
            }
        }

        #[test]
        fn player_exit_messages_report_failures_and_offline_blocks() {
            use std::os::windows::process::ExitStatusExt;
            let ok = PlayerReport {
                status: Some(ExitStatus::from_raw(0)),
                blocked: Some(3),
                ..Default::default()
            };
            let (message, success) = describe_exit(&ok, false);
            assert!(
                success && message.contains("blocked 3 network requests"),
                "{message}"
            );
            assert!(!describe_exit(&ok, true).0.contains("blocked"));
            let failed = PlayerReport {
                status: Some(ExitStatus::from_raw(1)),
                error_line: player_error(
                    r#"Error: Custom { kind: InvalidData, error: "this game is already open in a player window" }"#,
                ),
                ..Default::default()
            };
            let (message, success) = describe_exit(&failed, false);
            assert!(!success);
            assert_eq!(
                message,
                "The game could not open: this game is already open in a player window."
            );
            assert_eq!(
                player_error(r#"Error: "GameStrike player requires standard Windows integrity""#)
                    .unwrap(),
                "GameStrike player requires standard Windows integrity"
            );
            assert_eq!(
                player_error("GameStrike offline mode blocked a request"),
                None
            );
        }
    }
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    windows_launcher::run()
}
