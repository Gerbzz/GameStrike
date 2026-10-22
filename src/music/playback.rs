//! Source-only local-music transport, separate from catalog persistence and communication audio.
//! No decoder/output adapter is supplied here; construction never opens media or a device.
use super::{Track, validate_tracks};
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaybackState {
    #[default]
    Unavailable,
    Stopped,
    Loading,
    Starting,
    Playing,
    Pausing,
    Paused,
    Faulted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaybackCommand {
    Select(Uuid),
    /// Carries the intended track, so a failed/dropped Select cannot play another row.
    Play(Uuid),
    Pause,
    Stop,
    Next,
    Previous,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlaybackSnapshot {
    /// An adapter is attached; this does not certify a working output device.
    pub available: bool,
    pub state: PlaybackState,
    pub selected: Option<Uuid>,
    pub problem: Option<String>,
    pub revision: u64,
}

#[derive(Debug)]
struct Permit {
    cancelled: AtomicBool,
    operation: AtomicU64,
    output: AtomicBool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Authorization {
    pub(crate) epoch: u64,
    pub(crate) queue_revision: u64,
}

#[derive(Debug)]
struct RegisteredSource {
    track: Track,
    permit: Weak<Permit>,
}

#[derive(Debug, Default)]
struct Authority {
    queue_revision: u64,
    queue: Vec<Track>,
    registered: Vec<RegisteredSource>,
}

#[derive(Debug, Default)]
struct Fence {
    epoch: AtomicU64,
    closed: AtomicBool,
    authority: Mutex<Authority>,
}

/// Front-side revocation never waits for a decoder or backend call. The mutex
/// covers only bounded queue metadata and lease registration, never device work.
#[derive(Clone, Debug, Default)]
pub struct AbortHandle(Arc<Fence>);

impl AbortHandle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn abort(&self) -> Result<(), String> {
        let _authority = self.0.authority.lock().map_err(|_| self.poisoned())?;
        self.advance_epoch()?;
        Ok(())
    }

    pub fn close(&self) {
        self.0.closed.store(true, Ordering::SeqCst);
    }

    fn poisoned(&self) -> String {
        self.close();
        "Music authorization state is unavailable".into()
    }

    fn advance_epoch(&self) -> Result<(), String> {
        if self.0.closed.load(Ordering::SeqCst) {
            return Err("Music authorization is closed".into());
        }
        let previous = self.0.epoch.load(Ordering::SeqCst);
        let Some(next) = previous.checked_add(1) else {
            self.close();
            return Err("Music authorization counter is exhausted".into());
        };
        self.0.epoch.store(next, Ordering::SeqCst);
        Ok(())
    }

    pub(crate) fn authorization(&self) -> Result<Authorization, String> {
        let authority = self.0.authority.lock().map_err(|_| self.poisoned())?;
        if self.0.closed.load(Ordering::SeqCst) {
            return Err("Music authorization is closed".into());
        }
        Ok(Authorization {
            epoch: self.0.epoch.load(Ordering::SeqCst),
            queue_revision: authority.queue_revision,
        })
    }

    pub(crate) fn is_authorized(&self, expected: Authorization) -> bool {
        self.authorization()
            .is_ok_and(|current| current == expected)
    }

    pub(crate) fn update_queue(
        &self,
        queue: &[Track],
        replace: bool,
    ) -> Result<(Authorization, bool), String> {
        validate_tracks(queue)?;
        let mut authority = self.0.authority.lock().map_err(|_| self.poisoned())?;
        if self.0.closed.load(Ordering::SeqCst) {
            return Err("Music authorization is closed".into());
        }
        if !replace && authority.queue == queue {
            return Ok((
                Authorization {
                    epoch: self.0.epoch.load(Ordering::SeqCst),
                    queue_revision: authority.queue_revision,
                },
                false,
            ));
        }
        let Some(revision) = authority.queue_revision.checked_add(1) else {
            self.close();
            return Err("Music queue revision is exhausted".into());
        };
        if replace {
            self.advance_epoch()?;
        }
        let mut revoked = false;
        authority.registered.retain(|registered| {
            let Some(permit) = registered.permit.upgrade() else {
                return false;
            };
            if permit.cancelled.load(Ordering::SeqCst) {
                return false;
            }
            if replace
                || !queue.iter().any(|track| {
                    track.id == registered.track.id && track.path == registered.track.path
                })
            {
                permit.cancelled.store(true, Ordering::SeqCst);
                permit.output.store(false, Ordering::SeqCst);
                revoked = true;
                return false;
            }
            true
        });
        authority.queue = queue.to_vec();
        authority.queue_revision = revision;
        Ok((
            Authorization {
                epoch: self.0.epoch.load(Ordering::SeqCst),
                queue_revision: revision,
            },
            revoked,
        ))
    }

    fn register(
        &self,
        expected: Authorization,
        generation: u64,
        track: &Track,
    ) -> Result<PlaybackLease, String> {
        let mut authority = self.0.authority.lock().map_err(|_| self.poisoned())?;
        if self.0.closed.load(Ordering::SeqCst)
            || self.0.epoch.load(Ordering::SeqCst) != expected.epoch
            || authority.queue_revision != expected.queue_revision
            || !authority
                .queue
                .iter()
                .any(|allowed| allowed.id == track.id && allowed.path == track.path)
        {
            return Err("Music intent was superseded by newer control or queue state".into());
        }
        let permit = Arc::new(Permit {
            cancelled: AtomicBool::new(false),
            operation: AtomicU64::new(0),
            output: AtomicBool::new(false),
        });
        authority.registered.retain(|registered| {
            registered
                .permit
                .upgrade()
                .is_some_and(|permit| !permit.cancelled.load(Ordering::SeqCst))
        });
        authority.registered.push(RegisteredSource {
            track: track.clone(),
            permit: Arc::downgrade(&permit),
        });
        Ok(PlaybackLease {
            generation,
            permit,
            abort: self.clone(),
            epoch: expected.epoch,
        })
    }
}

/// Revocable authority for one decoded stream, distinct from its play/pause sequence.
/// Adapters must check cancellation before source work, `is_current` before applying
/// a queued operation, and `may_output` before enqueueing PCM/device writes. They must
/// also stop/flush their own bounded buffers on release. This cannot retract samples
/// already accepted by hardware, and is not proof of physical output cancellation.
#[derive(Clone, Debug)]
pub struct PlaybackLease {
    generation: u64,
    permit: Arc<Permit>,
    abort: AbortHandle,
    epoch: u64,
}

impl PlaybackLease {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn is_cancelled(&self) -> bool {
        self.permit.cancelled.load(Ordering::SeqCst)
            || self.abort.0.closed.load(Ordering::SeqCst)
            || self.abort.0.epoch.load(Ordering::SeqCst) != self.epoch
    }

    pub fn is_current(&self, operation: u64) -> bool {
        self.permit.operation.load(Ordering::SeqCst) == operation && !self.is_cancelled()
    }

    pub fn may_output(&self) -> bool {
        self.permit.output.load(Ordering::SeqCst) && !self.is_cancelled()
    }

    fn revoke(&self) {
        self.permit.cancelled.store(true, Ordering::SeqCst);
        self.permit.output.store(false, Ordering::SeqCst);
    }
}

#[derive(Clone, Debug)]
pub enum BackendEvent {
    Ready {
        generation: u64,
    },
    Started {
        generation: u64,
        operation: u64,
    },
    Paused {
        generation: u64,
        operation: u64,
    },
    /// Drained stream completion, including all queued PCM/output. This must NEVER
    /// mean decoder EOF while buffered samples remain: handling it releases the
    /// stream immediately. It belongs to the session, so pause/resume cannot stale it.
    Ended {
        generation: u64,
    },
    /// A terminal source/session/output failure, never a delayed per-operation
    /// rejection. An adapter must ignore superseded operations before side effects.
    Failed {
        generation: u64,
        error: String,
    },
}

impl BackendEvent {
    fn generation(&self) -> u64 {
        match self {
            Self::Ready { generation }
            | Self::Started { generation, .. }
            | Self::Paused { generation, .. }
            | Self::Ended { generation }
            | Self::Failed { generation, .. } => *generation,
        }
    }
}

/// Native-adapter boundary, not an implementation of audio output.
/// Calls must be nonblocking with bounded worker queues; decoding/file I/O belongs
/// on that adapter's worker, never a device callback. `load` prepares without output.
/// Success means a request was accepted; only matching events acknowledge it.
/// `release` is called after revocation and must invalidate/flush pending output.
/// `Ended` means PCM/output has drained, not merely that the decoder reached EOF.
/// No method may change device defaults or silently select a substitute endpoint.
/// A real host must keep Stop/revocation independent of blocking catalog scans.
pub trait PlaybackBackend: Send {
    fn load(&mut self, track: &Track, lease: PlaybackLease) -> Result<(), String>;
    fn play(&mut self, lease: &PlaybackLease, operation: u64) -> Result<(), String>;
    fn pause(&mut self, lease: &PlaybackLease, operation: u64) -> Result<(), String>;
    fn release(&mut self, lease: &PlaybackLease);
    fn poll(&mut self) -> Option<BackendEvent>;
}

pub struct PlaybackController {
    backend: Option<Box<dyn PlaybackBackend>>,
    queue: Vec<Track>,
    snapshot: PlaybackSnapshot,
    active: Option<PlaybackLease>,
    generation: u64,
    operation: u64,
    abort: AbortHandle,
    authorization: Authorization,
    pending_end: Option<u64>,
}

impl PlaybackController {
    pub fn unavailable() -> Self {
        Self {
            backend: None,
            queue: Vec::new(),
            snapshot: PlaybackSnapshot::default(),
            active: None,
            generation: 0,
            operation: 0,
            abort: AbortHandle::new(),
            authorization: Authorization {
                epoch: 0,
                queue_revision: 0,
            },
            pending_end: None,
        }
    }

    /// Attaches an idle adapter. It does not discover/open an endpoint or source.
    pub fn new(backend: Box<dyn PlaybackBackend>) -> Self {
        Self::with_abort_handle(backend, AbortHandle::new())
    }

    pub fn with_abort_handle(backend: Box<dyn PlaybackBackend>, abort: AbortHandle) -> Self {
        let authorization = abort.authorization().unwrap_or(Authorization {
            epoch: 0,
            queue_revision: 0,
        });
        Self {
            backend: Some(backend),
            queue: Vec::new(),
            snapshot: PlaybackSnapshot {
                available: true,
                state: PlaybackState::Stopped,
                ..Default::default()
            },
            active: None,
            generation: 0,
            operation: 0,
            abort,
            authorization,
            pending_end: None,
        }
    }

    pub fn abort_handle(&self) -> AbortHandle {
        self.abort.clone()
    }

    pub fn snapshot(&self) -> PlaybackSnapshot {
        self.snapshot.clone()
    }

    /// Explicit playlist/source replacement always revokes the former stream,
    /// even when the new saved order contains the same IDs and paths. No autoplay.
    pub fn replace_queue(&mut self, queue: &[Track]) -> Result<PlaybackSnapshot, String> {
        let (authorization, _) = self.abort.update_queue(queue, true)?;
        self.replace_queue_authorized(queue, authorization)
    }

    pub(crate) fn replace_queue_authorized(
        &mut self,
        queue: &[Track],
        authorization: Authorization,
    ) -> Result<PlaybackSnapshot, String> {
        self.authorize(authorization)?;
        validate_tracks(queue)?;
        self.release_active();
        self.snapshot.selected = self
            .snapshot
            .selected
            .filter(|id| queue.iter().any(|track| track.id == *id))
            .or_else(|| queue.first().map(|track| track.id));
        self.queue = queue.to_vec();
        self.set_idle(PlaybackState::Stopped);
        Ok(self.snapshot())
    }

    /// Ordinary add/remove/reorder edits preserve a retained ID's identical source.
    /// Removing/changing the selected source cancels, selects its successor and stops.
    pub fn reconcile_queue(&mut self, queue: &[Track]) -> Result<PlaybackSnapshot, String> {
        let (authorization, _) = self.abort.update_queue(queue, false)?;
        self.reconcile_queue_authorized(queue, authorization)
    }

    pub(crate) fn reconcile_queue_authorized(
        &mut self,
        queue: &[Track],
        authorization: Authorization,
    ) -> Result<PlaybackSnapshot, String> {
        self.authorize(authorization)?;
        validate_tracks(queue)?;
        if self.queue == queue {
            return Ok(self.snapshot());
        }
        let previous = self
            .snapshot
            .selected
            .and_then(|id| self.queue.iter().position(|track| track.id == id));
        let retained = previous.and_then(|index| {
            let old = &self.queue[index];
            queue
                .iter()
                .find(|track| track.id == old.id && track.path == old.path)
        });
        if retained.is_none() {
            self.release_active();
            self.snapshot.selected = queue
                .get(previous.unwrap_or(0).min(queue.len().saturating_sub(1)))
                .map(|track| track.id);
            self.set_idle(PlaybackState::Stopped);
        } else {
            self.changed();
        }
        self.queue = queue.to_vec();
        Ok(self.snapshot())
    }

    pub fn dispatch(&mut self, command: PlaybackCommand) -> Result<PlaybackSnapshot, String> {
        if command == PlaybackCommand::Stop {
            self.abort.abort()?;
        }
        let authorization = self.abort.authorization()?;
        self.dispatch_authorized(command, authorization)
    }

    pub(crate) fn dispatch_authorized(
        &mut self,
        command: PlaybackCommand,
        authorization: Authorization,
    ) -> Result<PlaybackSnapshot, String> {
        self.authorize(authorization)?;
        if command == PlaybackCommand::Stop {
            self.release_active();
            self.set_idle(PlaybackState::Stopped);
            return Ok(self.snapshot());
        }
        if self.backend.is_none() {
            return Err("Native music playback adapter is unavailable".into());
        }
        match command {
            PlaybackCommand::Select(id) => {
                self.track_index(id)?;
                if self.snapshot.selected != Some(id) {
                    self.release_active();
                    self.snapshot.selected = Some(id);
                    self.set_idle(PlaybackState::Stopped);
                }
            }
            PlaybackCommand::Play(id) => {
                self.track_index(id)?;
                if self.snapshot.selected != Some(id) || self.pending_end.is_some() {
                    self.start_load_id(id)?;
                } else {
                    match self.snapshot.state {
                        PlaybackState::Playing
                        | PlaybackState::Loading
                        | PlaybackState::Starting => {}
                        PlaybackState::Paused | PlaybackState::Pausing if self.active.is_some() => {
                            self.request_play()?;
                        }
                        _ => self.start_load()?,
                    }
                }
            }
            PlaybackCommand::Pause => match self.snapshot.state {
                _ if self.pending_end.is_some() => {
                    // A drained session cannot acknowledge Pause. Explicit input
                    // wins over deferred automatic advance without touching it.
                    self.release_active();
                    self.set_idle(PlaybackState::Paused);
                }
                PlaybackState::Loading | PlaybackState::Starting => {
                    self.release_active();
                    self.set_idle(PlaybackState::Paused);
                }
                PlaybackState::Playing => self.request_pause()?,
                _ => {}
            },
            PlaybackCommand::Next => self.navigate(true)?,
            PlaybackCommand::Previous => self.navigate(false)?,
            PlaybackCommand::Stop => unreachable!(),
        }
        Ok(self.snapshot())
    }

    /// Bounded work per host tick. State changes require current acknowledged events.
    pub fn poll(&mut self) -> bool {
        let Ok(authorization) = self.abort.authorization() else {
            return false;
        };
        self.poll_authorized(authorization)
    }

    pub(crate) fn poll_authorized(&mut self, authorization: Authorization) -> bool {
        if self.authorize(authorization).is_err() {
            return false;
        }
        let before = self.snapshot.revision;
        if self
            .active
            .as_ref()
            .is_some_and(PlaybackLease::is_cancelled)
        {
            self.release_active();
            self.set_idle(PlaybackState::Stopped);
        }
        if self.pending_end.is_some() {
            self.finish_ended();
        }
        for _ in 0..32 {
            let Some(event) = self.backend.as_mut().and_then(|backend| backend.poll()) else {
                break;
            };
            self.handle(event);
        }
        before != self.snapshot.revision
    }

    pub fn shutdown(&mut self) {
        self.abort.close();
        self.release_active();
        self.backend = None;
        self.set_idle(PlaybackState::Unavailable);
    }

    fn track_index(&self, id: Uuid) -> Result<usize, String> {
        self.queue
            .iter()
            .position(|track| track.id == id)
            .ok_or_else(|| "Track is not in the current music queue".into())
    }

    fn start_load(&mut self) -> Result<(), String> {
        let id = self
            .snapshot
            .selected
            .ok_or("Select a queued music track first")?;
        self.start_load_id(id)
    }

    fn start_load_id(&mut self, id: Uuid) -> Result<(), String> {
        let track = self.queue[self.track_index(id)?].clone();
        let Some(generation) = self.generation.checked_add(1) else {
            return self.fail("Music session counter is exhausted".into());
        };
        let lease = self
            .abort
            .register(self.authorization, generation, &track)?;
        self.release_active();
        self.generation = generation;
        self.snapshot.selected = Some(id);
        self.active = Some(lease.clone());
        self.snapshot.state = PlaybackState::Loading;
        self.snapshot.problem = None;
        self.changed();
        let result = self
            .backend
            .as_mut()
            .expect("attached adapter")
            .load(&track, lease);
        if let Err(error) = result {
            return self.fail(error);
        }
        Ok(())
    }

    fn next_operation(&mut self, output: bool) -> Result<(PlaybackLease, u64), String> {
        let Some(operation) = self.operation.checked_add(1) else {
            return self.fail("Music operation counter is exhausted".into());
        };
        let lease = self
            .active
            .as_ref()
            .ok_or("Music stream is no longer active")?
            .clone();
        self.operation = operation;
        // Revoke prior operation/output authority before dispatching its replacement.
        lease.permit.output.store(false, Ordering::SeqCst);
        lease.permit.operation.store(operation, Ordering::SeqCst);
        lease.permit.output.store(output, Ordering::SeqCst);
        Ok((lease, operation))
    }

    fn request_play(&mut self) -> Result<(), String> {
        let (lease, operation) = self.next_operation(true)?;
        self.snapshot.state = PlaybackState::Starting;
        self.snapshot.problem = None;
        self.changed();
        if let Err(error) = self
            .backend
            .as_mut()
            .expect("attached adapter")
            .play(&lease, operation)
        {
            return self.fail(error);
        }
        Ok(())
    }

    fn request_pause(&mut self) -> Result<(), String> {
        let (lease, operation) = self.next_operation(false)?;
        self.snapshot.state = PlaybackState::Pausing;
        self.changed();
        if let Err(error) = self
            .backend
            .as_mut()
            .expect("attached adapter")
            .pause(&lease, operation)
        {
            return self.fail(error);
        }
        Ok(())
    }

    fn navigate(&mut self, next: bool) -> Result<(), String> {
        let id = self
            .snapshot
            .selected
            .ok_or("Select a queued music track first")?;
        let index = self.track_index(id)?;
        let destination = if next {
            index
                .checked_add(1)
                .filter(|&index| index < self.queue.len())
        } else {
            index.checked_sub(1)
        }
        .ok_or("Already at that end of the music queue")?;
        let state = self.snapshot.state;
        let destination = self.queue[destination].id;
        match state {
            PlaybackState::Playing | PlaybackState::Loading | PlaybackState::Starting => {
                self.start_load_id(destination)
            }
            PlaybackState::Paused | PlaybackState::Pausing => {
                self.release_active();
                self.snapshot.selected = Some(destination);
                self.set_idle(PlaybackState::Paused);
                Ok(())
            }
            _ => {
                self.release_active();
                self.snapshot.selected = Some(destination);
                self.set_idle(PlaybackState::Stopped);
                Ok(())
            }
        }
    }

    fn handle(&mut self, event: BackendEvent) {
        let Some(lease) = self.active.as_ref() else {
            return;
        };
        if lease.is_cancelled() || lease.generation() != event.generation() {
            return;
        }
        match event {
            BackendEvent::Ready { .. } if self.snapshot.state == PlaybackState::Loading => {
                let _ = self.request_play();
            }
            BackendEvent::Started { operation, .. }
                if self.snapshot.state == PlaybackState::Starting
                    && lease.is_current(operation) =>
            {
                self.snapshot.state = PlaybackState::Playing;
                self.changed();
            }
            BackendEvent::Paused { operation, .. }
                if self.snapshot.state == PlaybackState::Pausing && lease.is_current(operation) =>
            {
                self.snapshot.state = PlaybackState::Paused;
                self.changed();
            }
            BackendEvent::Ended { generation } => {
                self.pending_end = Some(generation);
                self.finish_ended();
            }
            BackendEvent::Failed { error, .. } => {
                let _: Result<(), String> = self.fail(error);
            }
            _ => {}
        }
    }

    fn release_active(&mut self) {
        self.pending_end = None;
        if let Some(lease) = self.active.take() {
            lease.revoke();
            if let Some(backend) = &mut self.backend {
                backend.release(&lease);
            }
        }
    }

    fn set_idle(&mut self, state: PlaybackState) {
        self.snapshot.available = self.backend.is_some();
        self.snapshot.state = if self.snapshot.available {
            state
        } else {
            PlaybackState::Unavailable
        };
        self.snapshot.problem = None;
        self.changed();
    }

    fn fail<T>(&mut self, error: String) -> Result<T, String> {
        self.release_active();
        let error: String = error
            .chars()
            .filter(|character| !character.is_control())
            .take(512)
            .collect();
        let error = if error.trim().is_empty() {
            "Music playback failed".into()
        } else {
            error
        };
        self.snapshot.state = PlaybackState::Faulted;
        self.snapshot.problem = Some(error.clone());
        self.changed();
        Err(error)
    }

    fn changed(&mut self) {
        self.snapshot.revision = self.snapshot.revision.wrapping_add(1);
    }

    fn authorize(&mut self, authorization: Authorization) -> Result<(), String> {
        if !self.abort.is_authorized(authorization) {
            return Err("Music intent was superseded by newer control or queue state".into());
        }
        self.authorization = authorization;
        Ok(())
    }

    fn finish_ended(&mut self) {
        let Some(generation) = self.pending_end else {
            return;
        };
        let Some(lease) = &self.active else {
            self.pending_end = None;
            return;
        };
        if lease.generation() != generation || lease.is_cancelled() {
            self.pending_end = None;
            return;
        }
        let has_next = self
            .snapshot
            .selected
            .and_then(|id| self.queue.iter().position(|track| track.id == id))
            .is_some_and(|index| index + 1 < self.queue.len());
        if self.snapshot.state == PlaybackState::Playing && has_next {
            // A queue race leaves this completion pending for the latest committed
            // order. A genuine next-source failure already publishes Faulted.
            let _ = self.navigate(true);
        } else {
            self.release_active();
            self.set_idle(PlaybackState::Stopped);
        }
    }
}

impl Drop for PlaybackController {
    fn drop(&mut self) {
        self.abort.close();
        self.release_active();
    }
}
