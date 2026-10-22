//! Renderer-independent native application state. Only the worker owns filesystem/process hosts.
mod library;
pub use library::{DesktopHost, LibraryBackend};
use crate::music::{MusicCommand, MusicSnapshot, playback::PlaybackCommand};

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum GameId {
    Html(Uuid),
    Store(String),
}

#[derive(Clone, Debug)]
pub struct CoverImage {
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct GameCard {
    pub id: GameId,
    pub title: String,
    pub source: String,
    pub favorite: bool,
    pub files_ok: bool,
    pub last_played: Option<u64>,
    pub cover: Option<CoverImage>,
}

#[derive(Clone, Debug, Default)]
pub struct AppSnapshot {
    pub games: Vec<GameCard>,
    pub music: MusicSnapshot,
    pub problems: Vec<String>,
    /// Tracked process/helper lifetimes and short delegated-link guards; not store game telemetry.
    pub running: BTreeSet<GameId>,
    pub refreshing: bool,
    /// Catalog/cover revision. Music has its own revision and does not invalidate cover textures.
    pub revision: u64,
}

#[derive(Clone, Debug)]
pub enum AppCommand {
    Refresh,
    Launch(GameId),
    ToggleFavorite(GameId, bool),
    Import(PathBuf),
    Music(MusicCommand),
    /// Intercepted by the dedicated desktop playback service, never the catalog worker.
    Playback(PlaybackCommand),
    /// An intent intercepted by the desktop host; workers never own file-picker authority.
    ChooseMusicFiles,
}

/// A native/isolated-player process or OS handoff helper; not proof of a store game's lifetime.
pub trait RunningGame: Send {
    fn poll_exit(&mut self) -> Option<Result<(), String>>;
}

pub enum LaunchOutcome {
    Tracked(Box<dyn RunningGame>),
    /// Observe an OS opener's errors while retaining the same minimum handoff guard on every OS.
    HandoffProcess(Box<dyn RunningGame>),
    /// A store URI/shortcut was handed off. Its actual game lifetime is unknown.
    HandedOff,
}

pub trait CatalogBackend: Send {
    fn snapshot(&mut self) -> Result<AppSnapshot, String>;
    fn refresh(&mut self) -> Result<(), String>;
    fn import(&mut self, source: &Path) -> Result<(), String>;
    fn set_favorite(&mut self, id: &GameId, favorite: bool) -> Result<(), String>;
    fn launch(&mut self, id: &GameId) -> Result<LaunchOutcome, String>;
    fn update_music(&mut self, _command: MusicCommand) -> Result<MusicSnapshot, String> {
        Err("Local music queue is unavailable".into())
    }
}

enum Event {
    Snapshot {
        result: Result<AppSnapshot, String>,
        finishes_refresh: bool,
    },
    Ended(GameId, Result<(), String>),
    Music {
        result: Result<MusicSnapshot, String>,
        replaces_queue: bool,
    },
}

struct EventSender {
    sender: SyncSender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl EventSender {
    fn send(&self, event: Event) -> Result<(), ()> {
        self.sender.send(event).map_err(|_| ())?;
        (self.wake)();
        Ok(())
    }
}

/// Native UI facade. Reservation happens synchronously before the launch enters the worker queue.
pub struct LauncherService {
    snapshot: AppSnapshot,
    commands: Option<SyncSender<AppCommand>>,
    events: Option<Receiver<Event>>,
    shutdown: Arc<AtomicBool>,
    service_problems: Vec<String>,
    catalog_problems: Vec<String>,
    music_replacement_revision: u64,
}

impl LauncherService {
    pub fn start(
        factory: impl FnOnce() -> Result<Box<dyn CatalogBackend>, String> + Send + 'static,
    ) -> Self {
        Self::start_with_waker(factory, Arc::new(|| {}))
    }

    pub fn start_with_waker(
        factory: impl FnOnce() -> Result<Box<dyn CatalogBackend>, String> + Send + 'static,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let (commands, incoming) = mpsc::sync_channel(16);
        let (sender, events) = mpsc::sync_channel(16);
        let outgoing = EventSender { sender, wake };
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown);
        thread::spawn(move || match factory() {
            Ok(mut backend) => {
                if !worker_shutdown.load(Ordering::Acquire)
                    && outgoing
                        .send(Event::Snapshot {
                            result: backend.snapshot(),
                            finishes_refresh: true,
                        })
                        .is_ok()
                {
                    worker(backend, incoming, outgoing, worker_shutdown);
                }
            }
            Err(error) => {
                let _ = outgoing.send(Event::Snapshot {
                    result: Err(error),
                    finishes_refresh: true,
                });
            }
        });
        Self {
            snapshot: AppSnapshot {
                refreshing: true,
                ..Default::default()
            },
            commands: Some(commands),
            events: Some(events),
            shutdown,
            service_problems: Vec::new(),
            catalog_problems: Vec::new(),
            music_replacement_revision: 0,
        }
    }

    /// Owned generated covers and names; no filesystem, store, process or device authority exists.
    pub fn synthetic() -> Self {
        let titles = [
            "ORBIT RACER",
            "EMBER VALLEY",
            "NIGHT CIRCUIT",
            "SKYFORGE",
            "TIDAL RUN",
            "VECTOR RALLY",
            "IRON HORIZON",
            "STARBOUND ECHOES",
            "NEON DRIFT",
            "WILD FRONTIER",
            "FROSTLINE",
            "SOLAR ARENA",
        ];
        let games = titles
            .into_iter()
            .enumerate()
            .map(|(index, title)| {
                let mut rgba = Vec::with_capacity(96 * 144 * 4);
                for y in 0..144_usize {
                    for x in 0..96_usize {
                        let glow = ((x * 3 + y * 2 + index * 19) % 110) as u8;
                        let stripe = ((x + y / 2 + index * 7) % 35) < 4;
                        rgba.extend_from_slice(&[
                            if stripe { 240 } else { 30 + glow },
                            if stripe { 164 } else { 20 + glow / 2 },
                            25 + ((index * 27 + y) % 110) as u8,
                            255,
                        ]);
                    }
                }
                GameCard {
                    id: GameId::Html(Uuid::from_u128(index as u128 + 1)),
                    title: title.into(),
                    source: "OWNED SYNTHETIC FIXTURE".into(),
                    favorite: index % 4 == 0,
                    files_ok: true,
                    last_played: None,
                    cover: Some(CoverImage {
                        size: [96, 144],
                        rgba,
                    }),
                }
            })
            .collect();
        Self {
            snapshot: AppSnapshot {
                games,
                revision: 1,
                ..Default::default()
            },
            commands: None,
            events: None,
            shutdown: Arc::new(AtomicBool::new(false)),
            service_problems: Vec::new(),
            catalog_problems: Vec::new(),
            music_replacement_revision: 0,
        }
    }

    pub fn snapshot(&self) -> &AppSnapshot {
        &self.snapshot
    }

    /// Advances only after a successful explicit playlist load, even when IDs overlap.
    /// The dedicated transport uses this to distinguish replacement from ordinary edits.
    pub fn music_replacement_revision(&self) -> u64 {
        self.music_replacement_revision
    }

    pub fn report_problem(&mut self, message: String) {
        if !self.service_problems.contains(&message) {
            self.service_problems.push(message.clone());
            if self.service_problems.len() > 16 {
                self.service_problems.remove(0);
            }
        }
        self.snapshot.problems = self.catalog_problems.clone();
        for problem in &self.service_problems {
            if !self.snapshot.problems.contains(problem) {
                self.snapshot.problems.push(problem.clone());
            }
        }
    }

    pub fn dispatch(&mut self, command: AppCommand) -> Result<(), String> {
        let sender = self
            .commands
            .as_ref()
            .ok_or("Synthetic capture has no action authority")?;
        match &command {
            AppCommand::ChooseMusicFiles => {
                return Err("File selection belongs to the native desktop host".into());
            }
            AppCommand::Playback(_) => {
                return Err("Playback belongs to the dedicated music service".into());
            }
            AppCommand::Music(_) if !self.snapshot.music.available => {
                return Err("Local music queue is unavailable".into());
            }
            AppCommand::Launch(id) => {
                let game = self
                    .snapshot
                    .games
                    .iter()
                    .find(|game| &game.id == id)
                    .ok_or("Game is not in the current collection")?;
                if !game.files_ok {
                    return Err("Game files are missing".into());
                }
                if self.snapshot.running.contains(id) {
                    return Err("Game is already running or its launch is pending".into());
                }
            }
            AppCommand::Refresh if self.snapshot.refreshing => return Ok(()),
            AppCommand::ToggleFavorite(id, _)
                if !self.snapshot.games.iter().any(|game| &game.id == id) =>
            {
                return Err("Game is not in the current collection".into());
            }
            _ => {}
        }
        sender
            .try_send(command.clone())
            .map_err(|error| match error {
                TrySendError::Full(_) => "Library worker is busy; try again shortly".to_owned(),
                TrySendError::Disconnected(_) => "Library worker is unavailable".to_owned(),
            })?;
        match command {
            AppCommand::Launch(id) => {
                self.snapshot.running.insert(id);
            }
            AppCommand::Refresh => self.snapshot.refreshing = true,
            _ => {}
        }
        Ok(())
    }

    pub fn poll_events(&mut self) {
        // Bound work per native frame even when a worker finishes several operations together.
        for _ in 0..32 {
            let event = self
                .events
                .as_ref()
                .and_then(|events| events.try_recv().ok());
            let Some(event) = event else { break };
            match event {
                Event::Music {
                    result: Ok(music),
                    replaces_queue,
                } => {
                    if replaces_queue {
                        self.music_replacement_revision =
                            self.music_replacement_revision.wrapping_add(1);
                    }
                    self.snapshot.music = music;
                }
                Event::Music {
                    result: Err(error), ..
                } => self.report_problem(error),
                Event::Snapshot {
                    result: Ok(mut snapshot),
                    finishes_refresh,
                } => {
                    self.catalog_problems = snapshot.problems.clone();
                    snapshot.running = self.snapshot.running.clone();
                    snapshot.revision = self.snapshot.revision.wrapping_add(1);
                    snapshot.refreshing = self.snapshot.refreshing && !finishes_refresh;
                    for problem in &self.service_problems {
                        if !snapshot.problems.contains(problem) {
                            snapshot.problems.push(problem.clone());
                        }
                    }
                    self.snapshot = snapshot;
                }
                Event::Snapshot {
                    result: Err(error),
                    finishes_refresh,
                } => {
                    if finishes_refresh {
                        self.snapshot.refreshing = false;
                    }
                    self.report_problem(error);
                }
                Event::Ended(id, result) => {
                    self.snapshot.running.remove(&id);
                    if let Err(error) = result {
                        self.report_problem(error);
                    }
                }
            }
        }
    }
}

impl Drop for LauncherService {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
    }
}

enum ActiveLaunch {
    Process(Box<dyn RunningGame>),
    Delegated {
        until: Instant,
        helper: Option<Box<dyn RunningGame>>,
    },
}

fn worker(
    mut backend: Box<dyn CatalogBackend>,
    incoming: Receiver<AppCommand>,
    outgoing: EventSender,
    shutdown: Arc<AtomicBool>,
) {
    let mut active: Vec<(GameId, ActiveLaunch)> = Vec::new();
    loop {
        if shutdown.load(Ordering::Acquire) {
            return;
        }
        match incoming.recv_timeout(Duration::from_millis(40)) {
            Ok(AppCommand::Launch(id)) => {
                if shutdown.load(Ordering::Acquire) {
                    return;
                }
                // Defense in depth: UI reservation is not the only duplicate-process guard.
                if active.iter().any(|(running, _)| running == &id) {
                    continue;
                }
                match backend.launch(&id) {
                    Ok(LaunchOutcome::Tracked(process)) => {
                        active.push((id, ActiveLaunch::Process(process)))
                    }
                    Ok(LaunchOutcome::HandedOff) => active.push((
                        id,
                        ActiveLaunch::Delegated {
                            until: Instant::now() + Duration::from_secs(2),
                            helper: None,
                        },
                    )),
                    Ok(LaunchOutcome::HandoffProcess(helper)) => active.push((
                        id,
                        ActiveLaunch::Delegated {
                            until: Instant::now() + Duration::from_secs(2),
                            helper: Some(helper),
                        },
                    )),
                    Err(error) => {
                        if outgoing.send(Event::Ended(id, Err(error))).is_err() {
                            return;
                        }
                    }
                }
            }
            Ok(AppCommand::Music(command)) => {
                if shutdown.load(Ordering::Acquire) {
                    return;
                }
                // Publish only committed music state. Queue edits never reload covers/catalogs.
                let replaces_queue = matches!(&command, MusicCommand::LoadPlaylist(_));
                if outgoing
                    .send(Event::Music {
                        result: backend.update_music(command),
                        replaces_queue,
                    })
                    .is_err()
                {
                    return;
                }
            }
            Ok(command) => {
                if shutdown.load(Ordering::Acquire) {
                    return;
                }
                let finishes_refresh = matches!(&command, AppCommand::Refresh);
                let result = match command {
                    AppCommand::Refresh => backend.refresh(),
                    AppCommand::ToggleFavorite(id, favorite) => backend.set_favorite(&id, favorite),
                    AppCommand::Import(source) => backend.import(&source),
                    AppCommand::ChooseMusicFiles => {
                        Err("File selection is a desktop-only action".into())
                    }
                    AppCommand::Playback(_) => Err("Playback is a dedicated-service action".into()),
                    AppCommand::Music(_) => unreachable!(),
                    AppCommand::Launch(_) => unreachable!(),
                };
                let snapshot = result.and_then(|()| backend.snapshot());
                if outgoing
                    .send(Event::Snapshot {
                        result: snapshot,
                        finishes_refresh,
                    })
                    .is_err()
                {
                    return;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        let mut index = 0;
        while index < active.len() {
            let result = match &mut active[index].1 {
                ActiveLaunch::Process(process) => process.poll_exit(),
                ActiveLaunch::Delegated { until, helper } => {
                    let completion = helper.as_mut().and_then(|process| process.poll_exit());
                    match completion {
                        Some(Err(error)) => Some(Err(error)),
                        Some(Ok(())) => {
                            *helper = None;
                            (Instant::now() >= *until).then_some(Ok(()))
                        }
                        None if helper.is_none() => (Instant::now() >= *until).then_some(Ok(())),
                        None => None,
                    }
                }
            };
            if let Some(result) = result {
                let (id, _) = active.swap_remove(index);
                if outgoing.send(Event::Ended(id, result)).is_err() {
                    return;
                }
            } else {
                index += 1;
            }
        }
    }
}
