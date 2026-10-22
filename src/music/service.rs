//! Dedicated, bounded transport actor. No catalog scan, media decoder or device host lives here.
use super::{
    Track, validate_tracks,
    playback::{
        AbortHandle, Authorization, PlaybackBackend, PlaybackCommand, PlaybackController,
        PlaybackSnapshot, PlaybackState,
    },
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread::{self, Thread},
    time::Duration,
};

struct Request {
    authorization: Authorization,
    command: PlaybackCommand,
}

#[derive(Clone, Default)]
struct QueueUpdate {
    revision: u64,
    replace_revision: u64,
    tracks: Vec<Track>,
}

struct Event {
    authorization: Authorization,
    snapshot: PlaybackSnapshot,
}

struct Shared {
    abort: AbortHandle,
    queue: Mutex<QueueUpdate>,
    event: Mutex<Option<Event>>,
    closed: AtomicBool,
    finished: AtomicBool,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Shared {
    fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self {
            abort: AbortHandle::new(),
            queue: Mutex::new(QueueUpdate::default()),
            event: Mutex::new(None),
            closed: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            wake,
        }
    }

    fn publish(&self, snapshot: PlaybackSnapshot, authorization: Authorization) {
        if !self.abort.is_authorized(authorization) {
            return;
        }
        if let Ok(mut event) = self.event.lock() {
            // One latest snapshot, never a blocking/full event channel.
            *event = Some(Event {
                authorization,
                snapshot,
            });
        } else {
            self.abort.close();
        }
        (self.wake)();
    }
}

struct WorkerFinished(Arc<Shared>);
impl Drop for WorkerFinished {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::Release);
        (self.0.wake)();
    }
}

/// Native host facade. `unavailable` spawns nothing and has no transport authority.
/// Queue methods accept validated metadata only, even without an attached backend.
/// A real backend still requires separate adapter/device/runtime acceptance.
pub struct PlaybackService {
    snapshot: PlaybackSnapshot,
    queue: Vec<Track>,
    commands: Option<SyncSender<Request>>,
    shared: Arc<Shared>,
    worker: Option<Thread>,
}

impl PlaybackService {
    pub fn unavailable() -> Self {
        let shared = Arc::new(Shared::new(Arc::new(|| {})));
        shared.finished.store(true, Ordering::Release);
        Self {
            snapshot: PlaybackSnapshot::default(),
            queue: Vec::new(),
            commands: None,
            shared,
            worker: None,
        }
    }

    pub fn start_with_waker(
        factory: impl FnOnce() -> Result<Box<dyn PlaybackBackend>, String> + Send + 'static,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let shared = Arc::new(Shared::new(wake));
        let (commands, incoming) = mpsc::sync_channel(16);
        let owner = shared.clone();
        let worker = thread::spawn(move || run(factory, incoming, owner));
        let thread = worker.thread().clone();
        // Dropping a JoinHandle detaches; neither construction nor Drop waits for
        // an external backend. Front-side revocation remains independently usable.
        drop(worker);
        Self {
            snapshot: PlaybackSnapshot::default(),
            queue: Vec::new(),
            commands: Some(commands),
            shared,
            worker: Some(thread),
        }
    }

    pub fn snapshot(&self) -> &PlaybackSnapshot {
        &self.snapshot
    }

    pub fn dispatch(&mut self, command: PlaybackCommand) -> Result<(), String> {
        let sender = self
            .commands
            .as_ref()
            .ok_or("Native music playback adapter is unavailable")?;
        if self.shared.finished.load(Ordering::Acquire) {
            return Err("Native music playback worker is unavailable".into());
        }
        if command == PlaybackCommand::Stop {
            // Urgent revocation is not queued behind ordinary work or a scan.
            let result = self.shared.abort.abort();
            self.idle_front();
            self.unpark();
            return result;
        }
        if !self.snapshot.available {
            return Err("Native music playback adapter is not ready".into());
        }
        if let PlaybackCommand::Play(id) | PlaybackCommand::Select(id) = &command
            && !self.queue.iter().any(|track| track.id == *id)
        {
            return Err("Track is not in the current music queue".into());
        }
        let authorization = self.shared.abort.authorization()?;
        sender
            .try_send(Request {
                authorization,
                command,
            })
            .map_err(|error| match error {
                TrySendError::Full(_) => "Music worker is busy; try again shortly".to_owned(),
                TrySendError::Disconnected(_) => {
                    "Native music playback worker is unavailable".to_owned()
                }
            })?;
        self.unpark();
        Ok(())
    }

    pub fn replace_queue(&mut self, tracks: &[Track]) -> Result<(), String> {
        self.update_queue(tracks, true)
    }

    pub fn reconcile_queue(&mut self, tracks: &[Track]) -> Result<(), String> {
        self.update_queue(tracks, false)
    }

    fn update_queue(&mut self, tracks: &[Track], replace: bool) -> Result<(), String> {
        validate_tracks(tracks)?;
        if !replace && self.queue == tracks {
            return Ok(());
        }
        let previous = self
            .snapshot
            .selected
            .and_then(|id| self.queue.iter().position(|track| track.id == id));
        // Registration and authoritative path updates share a tiny lock. If a
        // pending load has not registered yet, its old queue ticket is rejected;
        // if it has, its permit is revoked here when removed/changed.
        let (authorization, revoked) = self.shared.abort.update_queue(tracks, replace)?;
        let mut pending = self.shared.queue.lock().map_err(|_| {
            self.shared.abort.close();
            "Music queue mailbox is unavailable".to_owned()
        })?;
        pending.revision = authorization.queue_revision;
        pending.tracks = tracks.to_vec();
        if replace {
            pending.replace_revision = authorization.queue_revision;
        }
        drop(pending);
        let retained = self
            .snapshot
            .selected
            .filter(|id| tracks.iter().any(|track| track.id == *id));
        self.snapshot.selected = retained.or_else(|| {
            if replace {
                tracks.first().map(|track| track.id)
            } else {
                tracks
                    .get(previous.unwrap_or(0).min(tracks.len().saturating_sub(1)))
                    .map(|track| track.id)
            }
        });
        self.queue = tracks.to_vec();
        if replace || revoked {
            self.idle_front();
        } else {
            self.snapshot.revision = self.snapshot.revision.wrapping_add(1);
        }
        self.unpark();
        Ok(())
    }

    pub fn poll_events(&mut self) {
        let event = match self.shared.event.lock() {
            Ok(mut event) => event.take(),
            Err(_) => {
                self.shared.abort.close();
                self.snapshot.available = false;
                self.snapshot.state = PlaybackState::Unavailable;
                self.snapshot.problem = Some("Music snapshot mailbox is unavailable".into());
                self.snapshot.revision = self.snapshot.revision.wrapping_add(1);
                return;
            }
        };
        if let Some(mut event) = event
            && self.shared.abort.is_authorized(event.authorization)
        {
            event.snapshot.revision = self.snapshot.revision.wrapping_add(1);
            self.snapshot = event.snapshot;
        }
        if self.commands.is_some()
            && self.shared.finished.load(Ordering::Acquire)
            && (self.snapshot.available
                || self.snapshot.state != PlaybackState::Unavailable
                || self.snapshot.problem.is_none())
        {
            self.snapshot.available = false;
            self.snapshot.state = PlaybackState::Unavailable;
            self.snapshot.problem.get_or_insert_with(|| {
                "Native music playback worker is unavailable".into()
            });
            self.snapshot.revision = self.snapshot.revision.wrapping_add(1);
        }
    }

    fn idle_front(&mut self) {
        self.snapshot.state = if self.snapshot.available {
            PlaybackState::Stopped
        } else {
            PlaybackState::Unavailable
        };
        if self.snapshot.available {
            self.snapshot.problem = None;
        }
        self.snapshot.revision = self.snapshot.revision.wrapping_add(1);
    }

    fn unpark(&self) {
        if let Some(worker) = &self.worker {
            worker.unpark();
        }
    }
}

impl Drop for PlaybackService {
    fn drop(&mut self) {
        // Neither the actor nor a partially completed load can gain fresh authority
        // after this. Do not join a backend that may be releasing external work.
        self.shared.abort.close();
        self.shared.closed.store(true, Ordering::Release);
        self.unpark();
    }
}

fn run(
    factory: impl FnOnce() -> Result<Box<dyn PlaybackBackend>, String>,
    incoming: Receiver<Request>,
    shared: Arc<Shared>,
) {
    let _finished = WorkerFinished(shared.clone());
    let backend = match factory() {
        Ok(backend) => backend,
        Err(error) => {
            if let Ok(authorization) = shared.abort.authorization() {
                shared.publish(
                    PlaybackSnapshot {
                        problem: Some(bounded_problem(error)),
                        ..Default::default()
                    },
                    authorization,
                );
            }
            return;
        }
    };
    let mut controller = PlaybackController::with_abort_handle(backend, shared.abort.clone());
    let mut queue_revision = 0;
    let mut stopped_epoch = 0;
    let mut deferred: Option<Request> = None;
    if let Ok(authorization) = shared.abort.authorization() {
        shared.publish(controller.snapshot(), authorization);
    }
    loop {
        if shared.closed.load(Ordering::Acquire) {
            return;
        }
        let mut changed = false;
        let pending = match shared.queue.lock() {
            Ok(pending) => pending.clone(),
            Err(_) => {
                shared.abort.close();
                return;
            }
        };
        // Queue state is authoritative independently of Stop epochs. Never discard
        // a committed queue just because an intervening Stop advanced its epoch.
        let Ok(authorization) = shared.abort.authorization() else { return };
        if pending.revision != authorization.queue_revision {
            thread::park_timeout(Duration::from_millis(20));
            continue;
        }
        // Stop is an epoch barrier, never an unversioned trailing flag that can
        // accidentally stop an explicitly newer Play on the following iteration.
        if stopped_epoch != authorization.epoch {
            if controller
                .dispatch_authorized(PlaybackCommand::Stop, authorization)
                .is_err()
            {
                continue;
            }
            stopped_epoch = authorization.epoch;
            changed = true;
        }
        if pending.revision != queue_revision {
            let result = if pending.replace_revision > queue_revision {
                controller.replace_queue_authorized(&pending.tracks, authorization)
            } else {
                controller.reconcile_queue_authorized(&pending.tracks, authorization)
            };
            if result.is_err() {
                continue;
            }
            queue_revision = pending.revision;
            changed = true;
        }
        let mut requested = false;
        let mut publish_request = false;
        let mut problem = None;
        let request = deferred
            .take()
            .map(Ok)
            .unwrap_or_else(|| incoming.try_recv());
        match request {
            Ok(request) => {
                requested = true;
                let Ok(current) = shared.abort.authorization() else { return };
                if request.authorization == current {
                    if current != authorization {
                        // It is a valid newer intent, not a stale request. Apply
                        // its control/queue barrier before executing it next turn.
                        deferred = Some(request);
                        continue;
                    }
                    let result = controller.dispatch_authorized(request.command, authorization);
                    if let Err(error) = result {
                        problem = Some(bounded_problem(error));
                    }
                    publish_request = true;
                }
            }
            Err(TryRecvError::Disconnected) => return,
            Err(TryRecvError::Empty) => {}
        }
        changed |= controller.poll_authorized(authorization);
        if changed || publish_request {
            let mut snapshot = controller.snapshot();
            // A terminal stream failure is more informative than a simultaneous
            // rejected command (for example Next at the end of the queue).
            snapshot.problem = snapshot.problem.or(problem);
            shared.publish(snapshot, authorization);
        }
        if !requested && !changed {
            thread::park_timeout(Duration::from_millis(20));
        }
    }
}

fn bounded_problem(error: String) -> String {
    let error: String = error
        .chars()
        .filter(|character| !character.is_control())
        .take(512)
        .collect();
    if error.trim().is_empty() {
        "Music playback failed".into()
    } else {
        error
    }
}
