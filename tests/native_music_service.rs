//! The injected backend owns no media or audio device; blocking is a cancellation probe.
use gamestrike::music::{
    Track,
    playback::{BackendEvent, PlaybackBackend, PlaybackCommand, PlaybackLease, PlaybackState},
    service::PlaybackService,
};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Default)]
struct Gate {
    released: Mutex<bool>,
    changed: Condvar,
    entered: AtomicBool,
}

impl Gate {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }

    fn wait(&self) {
        self.entered.store(true, Ordering::Release);
        let released = self.released.lock().unwrap();
        drop(self.changed.wait_while(released, |released| !*released).unwrap());
    }
}

struct ReleaseOnDrop(Arc<Gate>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[derive(Default)]
struct State {
    loads: Vec<(Track, PlaybackLease)>,
    plays: Vec<PlaybackLease>,
    released: Vec<(u64, bool)>,
    events: VecDeque<BackendEvent>,
    gate: Option<Arc<Gate>>,
    release_gate: Option<Arc<Gate>>,
    end_gate: Option<Arc<Gate>>,
    drained: bool,
    pauses: usize,
}

struct Backend(Arc<Mutex<State>>);
impl PlaybackBackend for Backend {
    fn load(&mut self, track: &Track, lease: PlaybackLease) -> Result<(), String> {
        let gate = {
            let mut state = self.0.lock().unwrap();
            state.loads.push((track.clone(), lease.clone()));
            state.drained = false;
            state.gate.take()
        };
        if let Some(gate) = gate {
            gate.wait();
        }
        self.0.lock().unwrap().events.push_back(BackendEvent::Ready {
            generation: lease.generation(),
        });
        Ok(())
    }

    fn play(&mut self, lease: &PlaybackLease, operation: u64) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        state.plays.push(lease.clone());
        state.events.push_back(BackendEvent::Started {
            generation: lease.generation(),
            operation,
        });
        Ok(())
    }

    fn pause(&mut self, lease: &PlaybackLease, operation: u64) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        state.pauses += 1;
        if state.drained {
            return Err("Fixture session has already drained".into());
        }
        state.events.push_back(BackendEvent::Paused {
            generation: lease.generation(),
            operation,
        });
        Ok(())
    }

    fn release(&mut self, lease: &PlaybackLease) {
        let gate = {
            let mut state = self.0.lock().unwrap();
            state.released.push((lease.generation(), lease.is_cancelled()));
            state.release_gate.take()
        };
        if let Some(gate) = gate {
            gate.wait();
        }
    }

    fn poll(&mut self) -> Option<BackendEvent> {
        let (event, gate) = {
            let mut state = self.0.lock().unwrap();
            let event = state.events.pop_front();
            let gate = if matches!(&event, Some(BackendEvent::Ended { .. })) {
                state.end_gate.take()
            } else {
                None
            };
            (event, gate)
        };
        if let Some(gate) = gate {
            gate.wait();
        }
        if matches!(&event, Some(BackendEvent::Ended { .. })) {
            self.0.lock().unwrap().drained = true;
        }
        event
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    tracks: Vec<Track>,
    state: Arc<Mutex<State>>,
    wakes: mpsc::Receiver<()>,
    service: PlaybackService,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let tracks: Vec<Track> = ["Zebra", "Moon", "Amber"]
            .into_iter()
            .enumerate()
            .map(|(index, title)| Track {
                id: Uuid::from_u128(index as u128 + 1),
                title: title.into(),
                path: temp.path().join(format!("{title}.wav")),
            })
            .collect();
        let state = Arc::new(Mutex::new(State::default()));
        let backend = state.clone();
        let (wake, wakes) = mpsc::channel();
        let service = PlaybackService::start_with_waker(
            move || Ok(Box::new(Backend(backend))),
            Arc::new(move || {
                let _ = wake.send(());
            }),
        );
        let mut fixture = Self {
            _temp: temp,
            tracks,
            state,
            wakes,
            service,
        };
        fixture.until(|fixture| fixture.service.snapshot().available);
        fixture.service.replace_queue(&fixture.tracks).unwrap();
        fixture.until(|fixture| fixture.service.snapshot().selected == Some(fixture.tracks[0].id));
        fixture
    }

    fn until(&mut self, predicate: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            self.service.poll_events();
            if predicate(self) {
                return;
            }
            assert!(Instant::now() < deadline, "transport worker did not publish expected state");
            let _ = self.wakes.recv_timeout(Duration::from_millis(20));
        }
    }

    fn play(&mut self, index: usize) -> PlaybackLease {
        self.service.dispatch(PlaybackCommand::Play(self.tracks[index].id)).unwrap();
        self.until(|fixture| fixture.service.snapshot().state == PlaybackState::Playing);
        self.state.lock().unwrap().loads.last().unwrap().1.clone()
    }

    fn blocked_load(&mut self, index: usize) -> (PlaybackLease, ReleaseOnDrop) {
        let gate = Arc::new(Gate::default());
        let release = ReleaseOnDrop(gate.clone());
        self.state.lock().unwrap().gate = Some(gate.clone());
        let count = self.state.lock().unwrap().loads.len();
        self.service.dispatch(PlaybackCommand::Play(self.tracks[index].id)).unwrap();
        self.until(|fixture| fixture.state.lock().unwrap().loads.len() > count);
        let lease = self.state.lock().unwrap().loads.last().unwrap().1.clone();
        (lease, release)
    }

    fn stop_without_backend_progress(&mut self) {
        let mut service = std::mem::replace(&mut self.service, PlaybackService::unavailable());
        let (done, completed) = mpsc::channel();
        std::thread::spawn(move || {
            let result = service.dispatch(PlaybackCommand::Stop);
            let _ = done.send((service, result));
        });
        let (service, result) = completed.recv_timeout(Duration::from_secs(3))
            .expect("Stop must not wait for backend progress");
        self.service = service;
        result.unwrap();
    }
}

#[test]
fn unavailable_service_has_no_worker_or_action_authority() {
    let mut service = PlaybackService::unavailable();
    assert!(!service.snapshot().available);
    assert_eq!(service.snapshot().state, PlaybackState::Unavailable);
    assert!(service.dispatch(PlaybackCommand::Stop).is_err());
    assert!(service.dispatch(PlaybackCommand::Play(Uuid::nil())).is_err());
    service.poll_events();
    assert_eq!(service.snapshot().state, PlaybackState::Unavailable);
}

#[test]
fn dedicated_worker_publishes_acknowledged_transport_and_wakes_the_native_host() {
    let mut fixture = Fixture::new();
    assert!(fixture.state.lock().unwrap().loads.is_empty());
    let lease = fixture.play(1);
    assert_eq!(fixture.service.snapshot().selected, Some(fixture.tracks[1].id));
    assert!(lease.may_output());
    fixture.service.dispatch(PlaybackCommand::Pause).unwrap();
    fixture.until(|fixture| fixture.service.snapshot().state == PlaybackState::Paused);
    assert!(!lease.may_output());
    fixture.service.dispatch(PlaybackCommand::Play(fixture.tracks[1].id)).unwrap();
    fixture.until(|fixture| fixture.service.snapshot().state == PlaybackState::Playing);
    assert_eq!(fixture.state.lock().unwrap().loads.len(), 1);
    fixture.stop_without_backend_progress();
    assert!(lease.is_cancelled());
    assert_eq!(fixture.service.snapshot().state, PlaybackState::Stopped);
}

#[test]
fn urgent_stop_revokes_during_a_blocked_load_and_ignores_its_late_completion() {
    let mut fixture = Fixture::new();
    let (lease, release) = fixture.blocked_load(0);
    fixture.stop_without_backend_progress();
    assert!(lease.is_cancelled());
    assert!(!lease.may_output());
    assert_eq!(fixture.service.snapshot().state, PlaybackState::Stopped);
    release.0.release();
    fixture.until(|fixture| !fixture.state.lock().unwrap().released.is_empty());
    fixture.service.poll_events();
    assert_eq!(fixture.service.snapshot().state, PlaybackState::Stopped);
    assert!(fixture.state.lock().unwrap().plays.is_empty());
    fixture.play(1);
    assert_eq!(fixture.service.snapshot().selected, Some(fixture.tracks[1].id));
}

#[test]
fn urgent_stop_is_accepted_when_the_ordinary_command_queue_is_full() {
    let mut fixture = Fixture::new();
    let (lease, release) = fixture.blocked_load(0);
    let mut full = false;
    for _ in 0..64 {
        if fixture.service.dispatch(PlaybackCommand::Play(fixture.tracks[0].id)).is_err() {
            full = true;
            break;
        }
    }
    assert!(full, "ordinary request capacity must be bounded");
    fixture.stop_without_backend_progress();
    assert!(lease.is_cancelled());
    release.0.release();
    fixture.until(|fixture| !fixture.state.lock().unwrap().released.is_empty());
    assert_eq!(fixture.service.snapshot().state, PlaybackState::Stopped);
    assert!(fixture.state.lock().unwrap().plays.is_empty());
}

#[test]
fn playlist_replacement_revokes_overlapping_sources_and_invalid_queue_does_not() {
    let mut fixture = Fixture::new();
    let lease = fixture.play(1);
    let duplicate = vec![fixture.tracks[0].clone(), fixture.tracks[0].clone()];
    assert!(fixture.service.replace_queue(&duplicate).is_err());
    assert!(fixture.service.reconcile_queue(&duplicate).is_err());
    assert!(!lease.is_cancelled());
    let mut saved = fixture.tracks.clone();
    saved.reverse();
    fixture.service.replace_queue(&saved).unwrap();
    assert!(lease.is_cancelled());
    fixture.until(|fixture| fixture.service.snapshot().state == PlaybackState::Stopped);
    assert_eq!(fixture.state.lock().unwrap().loads.len(), 1);
}

#[test]
fn queue_reorder_preserves_a_retained_source_but_current_removal_revokes_immediately() {
    let mut fixture = Fixture::new();
    let lease = fixture.play(1);
    let mut reordered = fixture.tracks.clone();
    reordered.swap(0, 1);
    fixture.service.reconcile_queue(&reordered).unwrap();
    assert!(!lease.is_cancelled());
    // Removing a different track must leave the retained stream authorized.
    reordered.pop();
    fixture.service.reconcile_queue(&reordered).unwrap();
    assert!(!lease.is_cancelled());
    assert!(lease.may_output());
    reordered.remove(0);
    fixture.service.reconcile_queue(&reordered).unwrap();
    assert!(lease.is_cancelled());
    fixture.until(|fixture| fixture.service.snapshot().selected == Some(fixture.tracks[0].id));
    assert_eq!(fixture.service.snapshot().state, PlaybackState::Stopped);
}

#[test]
fn drop_revokes_a_live_or_inflight_source_without_joining_the_backend() {
    for blocked in [false, true] {
        let mut fixture = Fixture::new();
        let (lease, release) = if blocked {
            let (lease, release) = fixture.blocked_load(0);
            (lease, Some(release))
        } else {
            (fixture.play(0), None)
        };
        let (done, completed) = mpsc::channel();
        std::thread::spawn(move || {
            drop(fixture.service);
            let _ = done.send(());
        });
        completed.recv_timeout(Duration::from_secs(3))
            .expect("Drop must not join a blocked backend");
        assert!(lease.is_cancelled());
        assert!(!lease.may_output());
        drop(release);
    }
}

#[test]
fn removing_an_unacknowledged_load_revokes_it_even_when_front_selection_is_older() {
    let mut fixture = Fixture::new();
    let (lease, release) = fixture.blocked_load(1);
    assert_eq!(fixture.service.snapshot().selected, Some(fixture.tracks[0].id));
    let remaining = vec![fixture.tracks[0].clone(), fixture.tracks[2].clone()];
    fixture.service.reconcile_queue(&remaining).unwrap();
    assert!(lease.is_cancelled());
    release.0.release();
    fixture.until(|fixture| !fixture.state.lock().unwrap().released.is_empty());
    assert!(fixture.state.lock().unwrap().plays.is_empty());
}

#[test]
fn a_stop_epoch_does_not_discard_the_latest_accepted_queue() {
    let mut fixture = Fixture::new();
    let (lease, release) = fixture.blocked_load(0);
    let latest = vec![fixture.tracks[2].clone()];
    fixture.service.replace_queue(&latest).unwrap();
    fixture.stop_without_backend_progress();
    assert!(lease.is_cancelled());
    release.0.release();
    fixture.until(|fixture| !fixture.state.lock().unwrap().released.is_empty());
    fixture.service.dispatch(PlaybackCommand::Play(fixture.tracks[2].id)).unwrap();
    fixture.until(|fixture| fixture.service.snapshot().state == PlaybackState::Playing);
    assert_eq!(fixture.service.snapshot().selected, Some(fixture.tracks[2].id));
    assert_eq!(fixture.state.lock().unwrap().loads.last().unwrap().0.id, fixture.tracks[2].id);
}

#[test]
fn a_new_play_after_stop_survives_an_older_release_finishing_late() {
    let mut fixture = Fixture::new();
    let lease = fixture.play(0);
    let gate = Arc::new(Gate::default());
    let release = ReleaseOnDrop(gate.clone());
    fixture.state.lock().unwrap().release_gate = Some(gate);
    fixture.stop_without_backend_progress();
    fixture.until(|fixture| !fixture.state.lock().unwrap().released.is_empty());
    assert!(lease.is_cancelled());
    fixture.stop_without_backend_progress();
    fixture.service.dispatch(PlaybackCommand::Play(fixture.tracks[1].id)).unwrap();
    release.0.release();
    fixture.until(|fixture| fixture.service.snapshot().state == PlaybackState::Playing);
    assert_eq!(fixture.service.snapshot().selected, Some(fixture.tracks[1].id));
    fixture.service.dispatch(PlaybackCommand::Pause).unwrap();
    fixture.until(|fixture| fixture.service.snapshot().state == PlaybackState::Paused);
    fixture.service.dispatch(PlaybackCommand::Play(fixture.tracks[1].id)).unwrap();
    fixture.until(|fixture| fixture.service.snapshot().state == PlaybackState::Playing);
}

#[test]
fn pause_wins_over_a_queue_raced_drained_completion_without_touching_expired_session() {
    let mut fixture = Fixture::new();
    let lease = fixture.play(0);
    let gate = Arc::new(Gate::default());
    let release = ReleaseOnDrop(gate.clone());
    {
        let mut state = fixture.state.lock().unwrap();
        state.end_gate = Some(gate.clone());
        state.events.push_back(BackendEvent::Ended {
            generation: lease.generation(),
        });
    }
    fixture.until(|_| gate.entered.load(Ordering::Acquire));
    let reordered = vec![
        fixture.tracks[0].clone(),
        fixture.tracks[2].clone(),
        fixture.tracks[1].clone(),
    ];
    fixture.service.reconcile_queue(&reordered).unwrap();
    fixture.service.dispatch(PlaybackCommand::Pause).unwrap();
    release.0.release();
    fixture.until(|fixture| fixture.service.snapshot().state == PlaybackState::Paused);
    assert_eq!(fixture.service.snapshot().selected, Some(fixture.tracks[0].id));
    assert_eq!(fixture.state.lock().unwrap().pauses, 0);
    assert_eq!(fixture.state.lock().unwrap().loads.len(), 1);
    assert!(lease.is_cancelled());
}

#[test]
fn explicit_play_wins_over_pending_completion_and_restarts_its_requested_track() {
    let mut fixture = Fixture::new();
    let old = fixture.play(0);
    let gate = Arc::new(Gate::default());
    let release = ReleaseOnDrop(gate.clone());
    {
        let mut state = fixture.state.lock().unwrap();
        state.end_gate = Some(gate.clone());
        state.events.push_back(BackendEvent::Ended {
            generation: old.generation(),
        });
    }
    fixture.until(|_| gate.entered.load(Ordering::Acquire));
    let reordered = vec![
        fixture.tracks[0].clone(),
        fixture.tracks[2].clone(),
        fixture.tracks[1].clone(),
    ];
    fixture.service.reconcile_queue(&reordered).unwrap();
    fixture.service.dispatch(PlaybackCommand::Play(fixture.tracks[0].id)).unwrap();
    release.0.release();
    fixture.until(|fixture| fixture.state.lock().unwrap().loads.len() == 2);
    fixture.until(|fixture| fixture.service.snapshot().state == PlaybackState::Playing);
    assert!(old.is_cancelled());
    assert_eq!(fixture.service.snapshot().selected, Some(fixture.tracks[0].id));
    assert_ne!(fixture.state.lock().unwrap().loads[1].1.generation(), old.generation());
}

#[test]
fn a_rejected_navigation_request_cannot_replace_terminal_output_failure_detail() {
    let mut fixture = Fixture::new();
    let lease = fixture.play(2);
    fixture.state.lock().unwrap().events.push_back(BackendEvent::Failed {
        generation: lease.generation(),
        error: "Fixture output disconnected".into(),
    });
    fixture.until(|fixture| fixture.service.snapshot().state == PlaybackState::Faulted);
    let before = fixture.service.snapshot().revision;
    fixture.service.dispatch(PlaybackCommand::Next).unwrap();
    fixture.until(|fixture| fixture.service.snapshot().revision > before);
    assert_eq!(fixture.service.snapshot().state, PlaybackState::Faulted);
    assert_eq!(fixture.service.snapshot().problem.as_deref(), Some("Fixture output disconnected"));
}

#[test]
fn failed_factory_is_bounded_unavailable_and_has_no_play_authority() {
    let (wake, wakes) = mpsc::channel();
    let mut service = PlaybackService::start_with_waker(
        || Err("x".repeat(900)),
        Arc::new(move || {
            let _ = wake.send(());
        }),
    );
    wakes.recv_timeout(Duration::from_secs(3)).unwrap();
    service.poll_events();
    assert!(!service.snapshot().available);
    assert_eq!(service.snapshot().state, PlaybackState::Unavailable);
    assert_eq!(service.snapshot().problem.as_ref().unwrap().chars().count(), 512);
    assert!(service.dispatch(PlaybackCommand::Play(Uuid::nil())).is_err());
    let stable = service.snapshot().clone();
    service.poll_events();
    service.poll_events();
    assert_eq!(service.snapshot(), &stable);
}
