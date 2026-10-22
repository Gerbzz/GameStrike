//! Deterministic controller contracts with owned path references, not device playback.
use gamestrike::music::{
    Track,
    playback::{
        BackendEvent, PlaybackBackend, PlaybackCommand, PlaybackController, PlaybackLease,
        PlaybackState,
    },
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Default)]
struct BackendState {
    loads: Vec<(Track, PlaybackLease)>,
    plays: Vec<(PlaybackLease, u64)>,
    pauses: Vec<(PlaybackLease, u64)>,
    releases: Vec<(u64, bool)>,
    events: VecDeque<BackendEvent>,
    fail_load: bool,
    fail_play: bool,
    fail_pause: bool,
}

struct Backend(Arc<Mutex<BackendState>>);

impl PlaybackBackend for Backend {
    fn load(&mut self, track: &Track, lease: PlaybackLease) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        state.loads.push((track.clone(), lease));
        if state.fail_load {
            Err("fixture decode failure".into())
        } else {
            Ok(())
        }
    }

    fn play(&mut self, lease: &PlaybackLease, operation: u64) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        state.plays.push((lease.clone(), operation));
        if state.fail_play {
            Err("fixture output failure".into())
        } else {
            Ok(())
        }
    }

    fn pause(&mut self, lease: &PlaybackLease, operation: u64) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        state.pauses.push((lease.clone(), operation));
        if state.fail_pause {
            Err("fixture pause failure".into())
        } else {
            Ok(())
        }
    }

    fn release(&mut self, lease: &PlaybackLease) {
        self.0
            .lock()
            .unwrap()
            .releases
            .push((lease.generation(), lease.is_cancelled()));
    }

    fn poll(&mut self) -> Option<BackendEvent> {
        self.0.lock().unwrap().events.pop_front()
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    queue: Vec<Track>,
    backend: Arc<Mutex<BackendState>>,
    controller: PlaybackController,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let queue: Vec<Track> = ["Zebra", "Moon", "Amber"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| Track {
                id: Uuid::from_u128(index as u128 + 1),
                title: name.into(),
                path: temp.path().join(format!("{name}.wav")),
            })
            .collect();
        let backend = Arc::new(Mutex::new(BackendState::default()));
        let mut controller = PlaybackController::new(Box::new(Backend(backend.clone())));
        controller.replace_queue(&queue).unwrap();
        Self {
            _temp: temp,
            queue,
            backend,
            controller,
        }
    }

    fn event(&mut self, event: BackendEvent) {
        self.backend.lock().unwrap().events.push_back(event);
        self.controller.poll();
    }

    fn lease(&self) -> PlaybackLease {
        self.backend.lock().unwrap().loads.last().unwrap().1.clone()
    }

    fn operation(&self) -> u64 {
        self.backend.lock().unwrap().plays.last().unwrap().1
    }

    fn start(&mut self, index: usize) -> PlaybackLease {
        self.controller
            .dispatch(PlaybackCommand::Play(self.queue[index].id))
            .unwrap();
        let lease = self.lease();
        self.event(BackendEvent::Ready {
            generation: lease.generation(),
        });
        self.event(BackendEvent::Started {
            generation: lease.generation(),
            operation: self.operation(),
        });
        assert_eq!(self.controller.snapshot().state, PlaybackState::Playing);
        lease
    }
}

#[test]
fn construction_queue_and_selection_do_not_open_or_start_a_source() {
    let mut fixture = Fixture::new();
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Stopped);
    fixture.controller.dispatch(PlaybackCommand::Select(fixture.queue[2].id)).unwrap();
    assert_eq!(fixture.controller.snapshot().selected, Some(fixture.queue[2].id));
    let state = fixture.backend.lock().unwrap();
    assert!(state.loads.is_empty());
    assert!(state.plays.is_empty());
    assert!(state.pauses.is_empty());
    assert!(state.releases.is_empty());
    assert!(fixture.queue.iter().all(|track| !track.path.exists()));
}

#[test]
fn explicit_play_id_and_acknowledgments_control_visible_playing_state() {
    let mut fixture = Fixture::new();
    fixture.controller.dispatch(PlaybackCommand::Select(fixture.queue[0].id)).unwrap();
    fixture.controller.dispatch(PlaybackCommand::Play(fixture.queue[2].id)).unwrap();
    let lease = fixture.lease();
    assert_eq!(fixture.controller.snapshot().selected, Some(fixture.queue[2].id));
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Loading);
    assert!(!lease.may_output());
    fixture.event(BackendEvent::Started { generation: lease.generation(), operation: 0 });
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Loading);
    fixture.event(BackendEvent::Ready { generation: lease.generation() });
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Starting);
    let operation = fixture.operation();
    assert!(lease.is_current(operation));
    assert!(lease.may_output());
    fixture.event(BackendEvent::Started { generation: lease.generation(), operation });
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Playing);
}

#[test]
fn unknown_play_or_select_preserves_the_active_session() {
    let mut fixture = Fixture::new();
    let lease = fixture.start(1);
    let before = fixture.controller.snapshot();
    assert!(fixture.controller.dispatch(PlaybackCommand::Select(Uuid::nil())).is_err());
    assert!(fixture.controller.dispatch(PlaybackCommand::Play(Uuid::nil())).is_err());
    assert_eq!(fixture.controller.snapshot(), before);
    assert!(!lease.is_cancelled());
    assert!(lease.may_output());
}

#[test]
fn pause_during_loading_cancels_then_explicit_play_uses_a_new_generation() {
    let mut fixture = Fixture::new();
    fixture.controller.dispatch(PlaybackCommand::Play(fixture.queue[0].id)).unwrap();
    let old = fixture.lease();
    fixture.controller.dispatch(PlaybackCommand::Pause).unwrap();
    assert!(old.is_cancelled());
    assert!(!old.may_output());
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Paused);
    fixture.event(BackendEvent::Ready { generation: old.generation() });
    assert!(fixture.backend.lock().unwrap().plays.is_empty());
    fixture.controller.dispatch(PlaybackCommand::Play(fixture.queue[0].id)).unwrap();
    assert_ne!(fixture.lease().generation(), old.generation());
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Loading);
}

#[test]
fn pause_during_starting_revokes_the_unacknowledged_play() {
    let mut fixture = Fixture::new();
    fixture.controller.dispatch(PlaybackCommand::Play(fixture.queue[0].id)).unwrap();
    let lease = fixture.lease();
    fixture.event(BackendEvent::Ready { generation: lease.generation() });
    let operation = fixture.operation();
    fixture.controller.dispatch(PlaybackCommand::Pause).unwrap();
    fixture.event(BackendEvent::Started { generation: lease.generation(), operation });
    assert!(lease.is_cancelled());
    assert!(!lease.is_current(operation));
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Paused);
}

#[test]
fn reordered_pause_and_resume_acknowledgments_cannot_override_latest_intent() {
    let mut fixture = Fixture::new();
    let lease = fixture.start(0);
    let old_play = fixture.operation();
    fixture.controller.dispatch(PlaybackCommand::Pause).unwrap();
    let pause = fixture.backend.lock().unwrap().pauses.last().unwrap().1;
    assert!(!lease.is_current(old_play));
    assert!(!lease.may_output());
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Pausing);
    fixture.controller.dispatch(PlaybackCommand::Play(fixture.queue[0].id)).unwrap();
    let resumed = fixture.operation();
    assert_ne!(resumed, pause);
    assert!(!lease.is_current(pause));
    fixture.event(BackendEvent::Paused { generation: lease.generation(), operation: pause });
    fixture.event(BackendEvent::Started { generation: lease.generation(), operation: old_play });
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Starting);
    fixture.event(BackendEvent::Started { generation: lease.generation(), operation: resumed });
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Playing);
    assert_eq!(fixture.backend.lock().unwrap().loads.len(), 1);
}

#[test]
fn stop_revokes_before_release_and_all_late_completions_are_ignored() {
    let mut fixture = Fixture::new();
    let lease = fixture.start(0);
    let operation = fixture.operation();
    fixture.controller.dispatch(PlaybackCommand::Stop).unwrap();
    let before = fixture.controller.snapshot();
    assert!(lease.is_cancelled());
    assert!(!lease.may_output());
    assert!(!lease.is_current(operation));
    assert_eq!(fixture.backend.lock().unwrap().releases, vec![(lease.generation(), true)]);
    for event in [
        BackendEvent::Ready { generation: lease.generation() },
        BackendEvent::Started { generation: lease.generation(), operation },
        BackendEvent::Paused { generation: lease.generation(), operation },
        BackendEvent::Ended { generation: lease.generation() },
        BackendEvent::Failed { generation: lease.generation(), error: "late failure".into() },
    ] {
        fixture.event(event);
        assert_eq!(fixture.controller.snapshot(), before);
    }
}

#[test]
fn ordered_navigation_has_no_wrap_and_only_active_play_intent_continues() {
    let mut fixture = Fixture::new();
    assert!(fixture.controller.dispatch(PlaybackCommand::Previous).is_err());
    fixture.controller.dispatch(PlaybackCommand::Next).unwrap();
    assert_eq!(fixture.controller.snapshot().selected, Some(fixture.queue[1].id));
    assert!(fixture.backend.lock().unwrap().loads.is_empty());
    let lease = fixture.start(1);
    fixture.controller.dispatch(PlaybackCommand::Next).unwrap();
    assert!(lease.is_cancelled());
    assert_eq!(fixture.controller.snapshot().selected, Some(fixture.queue[2].id));
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Loading);
    let before = fixture.controller.snapshot();
    assert!(fixture.controller.dispatch(PlaybackCommand::Next).is_err());
    assert_eq!(fixture.controller.snapshot(), before);
    fixture.controller.dispatch(PlaybackCommand::Pause).unwrap();
    let loads = fixture.backend.lock().unwrap().loads.len();
    fixture.controller.dispatch(PlaybackCommand::Previous).unwrap();
    assert_eq!(fixture.controller.snapshot().selected, Some(fixture.queue[1].id));
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Paused);
    assert_eq!(fixture.backend.lock().unwrap().loads.len(), loads);
}

#[test]
fn eos_uses_session_identity_across_pause_resume_and_stops_at_queue_end() {
    let mut fixture = Fixture::new();
    let first = fixture.start(1);
    fixture.controller.dispatch(PlaybackCommand::Pause).unwrap();
    let pause = fixture.backend.lock().unwrap().pauses.last().unwrap().1;
    fixture.event(BackendEvent::Paused { generation: first.generation(), operation: pause });
    fixture.controller.dispatch(PlaybackCommand::Play(fixture.queue[1].id)).unwrap();
    fixture.event(BackendEvent::Started {
        generation: first.generation(),
        operation: fixture.operation(),
    });
    fixture.event(BackendEvent::Ended { generation: first.generation() });
    assert!(first.is_cancelled());
    assert_eq!(fixture.controller.snapshot().selected, Some(fixture.queue[2].id));
    let last = fixture.lease();
    fixture.event(BackendEvent::Ready { generation: last.generation() });
    fixture.event(BackendEvent::Started {
        generation: last.generation(),
        operation: fixture.operation(),
    });
    fixture.event(BackendEvent::Ended { generation: last.generation() });
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Stopped);
    assert_eq!(fixture.controller.snapshot().selected, Some(fixture.queue[2].id));
    assert_eq!(fixture.backend.lock().unwrap().loads.len(), 2);
}

#[test]
fn eos_while_pause_is_pending_does_not_autoadvance() {
    let mut fixture = Fixture::new();
    let lease = fixture.start(0);
    fixture.controller.dispatch(PlaybackCommand::Pause).unwrap();
    fixture.event(BackendEvent::Ended { generation: lease.generation() });
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Stopped);
    assert_eq!(fixture.controller.snapshot().selected, Some(fixture.queue[0].id));
    assert_eq!(fixture.backend.lock().unwrap().loads.len(), 1);
}

#[test]
fn eos_advance_preserves_a_next_source_failure_instead_of_reporting_normal_stop() {
    let mut fixture = Fixture::new();
    let previous = fixture.start(0);
    fixture.backend.lock().unwrap().fail_load = true;
    fixture.event(BackendEvent::Ended { generation: previous.generation() });
    assert!(previous.is_cancelled());
    assert!(fixture.lease().is_cancelled());
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Faulted);
    assert_eq!(fixture.controller.snapshot().selected, Some(fixture.queue[1].id));
    assert_eq!(fixture.controller.snapshot().problem.as_deref(), Some("fixture decode failure"));
}

#[test]
fn load_failure_and_current_disconnect_fail_closed_then_allow_explicit_retry() {
    let mut fixture = Fixture::new();
    fixture.backend.lock().unwrap().fail_load = true;
    assert!(fixture.controller.dispatch(PlaybackCommand::Play(fixture.queue[0].id)).is_err());
    let failed = fixture.lease();
    assert!(failed.is_cancelled());
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Faulted);
    fixture.backend.lock().unwrap().fail_load = false;
    let live = fixture.start(0);
    fixture.event(BackendEvent::Failed { generation: live.generation(), error: "x".repeat(900) });
    assert!(live.is_cancelled());
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Faulted);
    assert_eq!(fixture.controller.snapshot().problem.unwrap().chars().count(), 512);
    let retry = fixture.start(0);
    assert_ne!(retry.generation(), live.generation());
    assert!(fixture.controller.snapshot().problem.is_none());
}

#[test]
fn play_or_pause_request_failure_revokes_the_source() {
    for during_pause in [false, true] {
        let mut fixture = Fixture::new();
        let lease = if during_pause {
            fixture.start(0)
        } else {
            fixture.controller.dispatch(PlaybackCommand::Play(fixture.queue[0].id)).unwrap();
            fixture.lease()
        };
        if during_pause {
            fixture.backend.lock().unwrap().fail_pause = true;
            assert!(fixture.controller.dispatch(PlaybackCommand::Pause).is_err());
        } else {
            fixture.backend.lock().unwrap().fail_play = true;
            fixture.event(BackendEvent::Ready { generation: lease.generation() });
        }
        assert!(lease.is_cancelled());
        assert!(!lease.may_output());
        assert_eq!(fixture.controller.snapshot().state, PlaybackState::Faulted);
    }
}

#[test]
fn ordinary_reorder_preserves_session_but_removed_or_changed_source_cancels() {
    let mut fixture = Fixture::new();
    let first = fixture.start(1);
    let mut reordered = fixture.queue.clone();
    reordered.swap(0, 1);
    fixture.controller.reconcile_queue(&reordered).unwrap();
    assert!(!first.is_cancelled());
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Playing);
    reordered.remove(0);
    fixture.controller.reconcile_queue(&reordered).unwrap();
    assert!(first.is_cancelled());
    assert_eq!(fixture.controller.snapshot().selected, Some(fixture.queue[0].id));
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Stopped);
    fixture.start(0);
    let second = fixture.lease();
    let mut changed = reordered;
    changed[0].path = fixture._temp.path().join("replacement.wav");
    fixture.controller.reconcile_queue(&changed).unwrap();
    assert!(second.is_cancelled());
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Stopped);
}

#[test]
fn explicit_playlist_replacement_cancels_even_overlapping_ids_without_autoplay() {
    let mut fixture = Fixture::new();
    let lease = fixture.start(1);
    let mut saved = fixture.queue.clone();
    saved.reverse();
    fixture.controller.replace_queue(&saved).unwrap();
    assert!(lease.is_cancelled());
    assert_eq!(fixture.controller.snapshot().selected, Some(fixture.queue[1].id));
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Stopped);
    assert_eq!(fixture.backend.lock().unwrap().loads.len(), 1);
}

#[test]
fn invalid_queue_is_transactional_and_empty_queue_releases_current_source() {
    let mut fixture = Fixture::new();
    let lease = fixture.start(0);
    let before = fixture.controller.snapshot();
    let duplicate = vec![fixture.queue[0].clone(), fixture.queue[0].clone()];
    assert!(fixture.controller.replace_queue(&duplicate).is_err());
    assert!(fixture.controller.reconcile_queue(&duplicate).is_err());
    assert_eq!(fixture.controller.snapshot(), before);
    assert!(!lease.is_cancelled());
    fixture.controller.reconcile_queue(&[]).unwrap();
    assert!(lease.is_cancelled());
    assert_eq!(fixture.controller.snapshot().selected, None);
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Stopped);
}

#[test]
fn unavailable_shutdown_and_drop_never_leave_a_live_lease() {
    let mut unavailable = PlaybackController::unavailable();
    assert!(!unavailable.snapshot().available);
    assert_eq!(unavailable.snapshot().state, PlaybackState::Unavailable);
    assert!(unavailable.dispatch(PlaybackCommand::Play(Uuid::nil())).is_err());
    unavailable.dispatch(PlaybackCommand::Stop).unwrap();
    for shutdown in [false, true] {
        let mut fixture = Fixture::new();
        let lease = fixture.start(0);
        if shutdown {
            fixture.controller.shutdown();
            assert!(!fixture.controller.snapshot().available);
            assert!(fixture.controller.dispatch(PlaybackCommand::Play(fixture.queue[0].id)).is_err());
            assert!(!fixture.controller.poll());
        }
        drop(fixture.controller);
        assert!(lease.is_cancelled());
        assert!(!lease.may_output());
        assert_eq!(fixture.backend.lock().unwrap().releases, vec![(lease.generation(), true)]);
    }
}
