//! All PCM is owned test data. These tests open no source files or audio devices.
use gamestrike::music::{
    Track,
    playback::{
        BackendEvent, PlaybackBackend, PlaybackCommand, PlaybackController, PlaybackLease,
        PlaybackState,
    },
    streaming::{
        DecodedRead, Decoder, DecoderFactory, MAX_BUFFER_FRAMES, Output, OutputFactory,
        OutputState, OutputWrite, StreamingBackend,
    },
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Default)]
struct Memory {
    reads: VecDeque<Result<DecodedRead, String>>,
    writes: VecDeque<Result<OutputWrite, String>>,
    accepted: Vec<[f32; 2]>,
    read_limits: Vec<usize>,
    loads: Vec<(Track, PlaybackLease)>,
    output_rates: Vec<u32>,
    requests: Vec<(u64, u64, OutputState)>,
    acknowledged: Option<(u64, u64, OutputState)>,
    drained: bool,
    fault: Option<&'static str>,
    clears: usize,
    output_releases: usize,
    decoder_releases: usize,
}

struct MemoryDecoderFactory(Arc<Mutex<Memory>>);
struct MemoryDecoder(Arc<Mutex<Memory>>);
struct MemoryOutputFactory(Arc<Mutex<Memory>>);
struct MemoryOutput(Arc<Mutex<Memory>>);

impl DecoderFactory for MemoryDecoderFactory {
    fn begin(&mut self, track: &Track, lease: PlaybackLease) -> Result<Box<dyn Decoder>, String> {
        let mut memory = self.0.lock().unwrap();
        memory.loads.push((track.clone(), lease));
        if memory.fault == Some("decoder begin") {
            return Err("decoder begin failed".into());
        }
        Ok(Box::new(MemoryDecoder(self.0.clone())))
    }
}

impl Decoder for MemoryDecoder {
    fn poll(&mut self, max_frames: usize) -> Result<DecodedRead, String> {
        let mut memory = self.0.lock().unwrap();
        memory.read_limits.push(max_frames);
        if memory.fault == Some("decode") {
            return Err("decode failed".into());
        }
        memory.reads.pop_front().unwrap_or(Ok(DecodedRead::Pending))
    }

    fn release(&mut self) {
        self.0.lock().unwrap().decoder_releases += 1;
    }
}

impl OutputFactory for MemoryOutputFactory {
    fn begin(&mut self, sample_rate: u32, _: PlaybackLease) -> Result<Box<dyn Output>, String> {
        let mut memory = self.0.lock().unwrap();
        memory.output_rates.push(sample_rate);
        if memory.fault == Some("output begin") {
            return Err("output begin failed".into());
        }
        Ok(Box::new(MemoryOutput(self.0.clone())))
    }
}

impl Output for MemoryOutput {
    fn start(&mut self, lease: &PlaybackLease, operation: u64) -> Result<(), String> {
        let mut memory = self.0.lock().unwrap();
        memory
            .requests
            .push((lease.generation(), operation, OutputState::Running));
        if memory.fault == Some("start") {
            return Err("start failed".into());
        }
        Ok(())
    }

    fn pause(&mut self, lease: &PlaybackLease, operation: u64) -> Result<(), String> {
        let mut memory = self.0.lock().unwrap();
        memory
            .requests
            .push((lease.generation(), operation, OutputState::Paused));
        if memory.fault == Some("pause") {
            return Err("pause failed".into());
        }
        Ok(())
    }

    fn poll_state(&mut self, lease: &PlaybackLease, operation: u64) -> Result<OutputState, String> {
        let memory = self.0.lock().unwrap();
        if memory.fault == Some("state") {
            return Err("state failed".into());
        }
        Ok(match memory.acknowledged {
            Some((generation, op, state))
                if generation == lease.generation() && op == operation =>
            {
                state
            }
            _ => OutputState::Pending,
        })
    }

    fn try_write(&mut self, frames: &[[f32; 2]]) -> Result<OutputWrite, String> {
        let mut memory = self.0.lock().unwrap();
        if memory.fault == Some("write") {
            return Err("write failed".into());
        }
        let result = memory
            .writes
            .pop_front()
            .unwrap_or(Ok(OutputWrite::Written(frames.len())));
        if let Ok(OutputWrite::Written(count)) = &result
            && *count > 0
            && *count <= frames.len()
        {
            memory.accepted.extend_from_slice(&frames[..*count]);
            memory.drained = false;
        }
        result
    }

    fn drained(&mut self) -> Result<bool, String> {
        let memory = self.0.lock().unwrap();
        if memory.fault == Some("drain") {
            return Err("drain failed".into());
        }
        Ok(memory.drained)
    }

    fn clear(&mut self) {
        self.0.lock().unwrap().clears += 1;
    }

    fn release(&mut self) {
        self.0.lock().unwrap().output_releases += 1;
    }
}

// Only the production controller can mint/revoke leases. This relay obtains genuine
// authority without exposing private lease fields or manufacturing permissions.
#[derive(Default)]
struct Authority {
    leases: Vec<PlaybackLease>,
    plays: Vec<u64>,
    pauses: Vec<u64>,
    events: VecDeque<BackendEvent>,
}

struct Relay(Arc<Mutex<Authority>>);

impl PlaybackBackend for Relay {
    fn load(&mut self, _: &Track, lease: PlaybackLease) -> Result<(), String> {
        self.0.lock().unwrap().leases.push(lease);
        Ok(())
    }

    fn play(&mut self, _: &PlaybackLease, operation: u64) -> Result<(), String> {
        self.0.lock().unwrap().plays.push(operation);
        Ok(())
    }

    fn pause(&mut self, _: &PlaybackLease, operation: u64) -> Result<(), String> {
        self.0.lock().unwrap().pauses.push(operation);
        Ok(())
    }

    fn release(&mut self, _: &PlaybackLease) {}

    fn poll(&mut self) -> Option<BackendEvent> {
        self.0.lock().unwrap().events.pop_front()
    }
}

struct Fixture {
    memory: Arc<Mutex<Memory>>,
    authority: Arc<Mutex<Authority>>,
    stream: StreamingBackend<MemoryDecoderFactory, MemoryOutputFactory>,
    controller: PlaybackController,
    track: Track,
}

impl Fixture {
    fn new(reads: Vec<DecodedRead>) -> Self {
        let memory = Arc::new(Mutex::new(Memory {
            reads: reads.into_iter().map(Ok).collect(),
            ..Default::default()
        }));
        let authority = Arc::new(Mutex::new(Authority::default()));
        let stream = StreamingBackend::new(
            MemoryDecoderFactory(memory.clone()),
            MemoryOutputFactory(memory.clone()),
        );
        let track = Track {
            id: Uuid::from_u128(1),
            title: "Owned stereo fixture".into(),
            path: std::env::temp_dir().join("gamestrike-streaming-fixture.wav"),
        };
        let mut controller = PlaybackController::new(Box::new(Relay(authority.clone())));
        controller
            .replace_queue(std::slice::from_ref(&track))
            .unwrap();
        Self {
            memory,
            authority,
            stream,
            controller,
            track,
        }
    }

    fn load(&mut self) -> PlaybackLease {
        self.controller
            .dispatch(PlaybackCommand::Play(self.track.id))
            .unwrap();
        let lease = self
            .authority
            .lock()
            .unwrap()
            .leases
            .last()
            .unwrap()
            .clone();
        self.stream.load(&self.track, lease.clone()).unwrap();
        lease
    }

    fn deliver(&mut self, event: BackendEvent) {
        self.authority.lock().unwrap().events.push_back(event);
        self.controller.poll();
    }

    fn play_operation(&self) -> u64 {
        *self.authority.lock().unwrap().plays.last().unwrap()
    }

    fn acknowledge(&self, lease: &PlaybackLease, operation: u64, state: OutputState) {
        self.memory.lock().unwrap().acknowledged = Some((lease.generation(), operation, state));
    }

    fn prime(&mut self) -> PlaybackLease {
        let lease = self.load();
        let ready = self.stream.poll().expect("first valid PCM must prime");
        assert!(
            matches!(ready, BackendEvent::Ready { generation } if generation == lease.generation())
        );
        assert!(self.memory.lock().unwrap().output_rates.is_empty());
        self.deliver(ready);
        self.stream.play(&lease, self.play_operation()).unwrap();
        lease
    }

    fn start(&mut self) -> PlaybackLease {
        let lease = self.prime();
        self.acknowledge(&lease, self.play_operation(), OutputState::Running);
        let started = self.stream.poll().expect("acknowledged output start");
        assert!(
            matches!(started, BackendEvent::Started { generation, operation }
            if generation == lease.generation() && operation == self.play_operation())
        );
        self.deliver(started);
        assert_eq!(self.controller.snapshot().state, PlaybackState::Playing);
        lease
    }

    fn pause(&mut self, lease: &PlaybackLease) -> u64 {
        self.controller.dispatch(PlaybackCommand::Pause).unwrap();
        let operation = *self.authority.lock().unwrap().pauses.last().unwrap();
        self.stream.pause(lease, operation).unwrap();
        operation
    }

    fn failure(&mut self, generation: u64) {
        let event = self.stream.poll().expect("terminal failure event");
        assert!(
            matches!(&event, BackendEvent::Failed { generation: actual, error }
            if *actual == generation && !error.is_empty())
        );
        self.deliver(event);
        assert_eq!(self.controller.snapshot().state, PlaybackState::Faulted);
        assert_eq!(self.stream.buffered_frames(), 0);
        assert!(self.stream.poll().is_none());
    }
}

fn block(frames: &[[f32; 2]]) -> DecodedRead {
    DecodedRead::Frames {
        sample_rate: 44_100,
        frames: frames.to_vec(),
    }
}

#[test]
fn construction_is_idle_and_loading_primes_without_opening_output() {
    let mut fixture = Fixture::new(vec![block(&[[0.1, -0.2], [0.3, -0.4]])]);
    assert!(fixture.stream.poll().is_none());
    assert!(fixture.memory.lock().unwrap().loads.is_empty());
    let lease = fixture.load();
    assert!(!lease.may_output());
    let ready = fixture.stream.poll().unwrap();
    assert!(
        matches!(ready, BackendEvent::Ready { generation } if generation == lease.generation())
    );
    assert_eq!(fixture.stream.buffered_frames(), 2);
    for _ in 0..3 {
        assert!(fixture.stream.poll().is_none());
    }
    let memory = fixture.memory.lock().unwrap();
    assert!(memory.output_rates.is_empty());
    assert!(memory.accepted.is_empty());
    assert_eq!(memory.read_limits, vec![MAX_BUFFER_FRAMES]);
}

#[test]
fn pending_decode_has_no_false_ready_or_output() {
    let mut fixture = Fixture::new(vec![DecodedRead::Pending, block(&[[0.25, -0.5]])]);
    fixture.load();
    assert!(fixture.stream.poll().is_none());
    assert_eq!(fixture.stream.buffered_frames(), 0);
    assert!(fixture.memory.lock().unwrap().output_rates.is_empty());
    assert!(matches!(
        fixture.stream.poll(),
        Some(BackendEvent::Ready { .. })
    ));
}

#[test]
fn stereo_order_survives_partial_writes_and_backpressure_without_decode_ahead() {
    let first = [[0.1, -0.1], [0.2, -0.3], [0.4, -0.5]];
    let second = [[-0.6, 0.7], [-0.8, 0.9]];
    let mut fixture = Fixture::new(vec![block(&first), block(&second), DecodedRead::End]);
    fixture.memory.lock().unwrap().writes = VecDeque::from([
        Ok(OutputWrite::Written(1)),
        Ok(OutputWrite::Pending),
        Ok(OutputWrite::Written(2)),
    ]);
    fixture.start();
    assert!(fixture.stream.poll().is_none());
    assert_eq!(fixture.stream.buffered_frames(), 2);
    assert!(fixture.stream.poll().is_none());
    assert_eq!(fixture.memory.lock().unwrap().accepted, first[..1]);
    assert_eq!(fixture.memory.lock().unwrap().read_limits.len(), 1);
    assert!(fixture.stream.poll().is_none());
    assert_eq!(fixture.memory.lock().unwrap().accepted, first);
    assert!(fixture.stream.poll().is_none());
    let mut expected = first.to_vec();
    expected.extend_from_slice(&second);
    let memory = fixture.memory.lock().unwrap();
    assert_eq!(memory.accepted, expected);
    assert_eq!(memory.output_rates, vec![44_100]);
    assert_eq!(memory.read_limits, vec![MAX_BUFFER_FRAMES; 2]);
    assert!(fixture.stream.buffered_frames() <= MAX_BUFFER_FRAMES);
}

#[test]
fn output_acknowledgments_gate_events_and_pause_preserves_pcm_position() {
    let frames = [[0.1, 0.8], [0.2, 0.7], [0.3, 0.6]];
    let mut fixture = Fixture::new(vec![block(&frames), DecodedRead::End]);
    let lease = fixture.prime();
    assert!(fixture.stream.poll().is_none());
    assert!(fixture.memory.lock().unwrap().accepted.is_empty());
    fixture.acknowledge(&lease, fixture.play_operation(), OutputState::Running);
    let started = fixture.stream.poll().unwrap();
    fixture.deliver(started);
    fixture
        .memory
        .lock()
        .unwrap()
        .writes
        .push_back(Ok(OutputWrite::Written(1)));
    assert!(fixture.stream.poll().is_none());
    let operation = fixture.pause(&lease);
    assert!(!lease.may_output());
    assert!(fixture.stream.poll().is_none()); // Running is not a pause acknowledgement.
    fixture.acknowledge(&lease, operation, OutputState::Paused);
    let paused = fixture.stream.poll().unwrap();
    assert!(matches!(paused, BackendEvent::Paused { operation: op, .. } if op == operation));
    fixture.deliver(paused);
    for _ in 0..4 {
        assert!(fixture.stream.poll().is_none());
    }
    assert_eq!(fixture.stream.buffered_frames(), 2);
    assert_eq!(fixture.memory.lock().unwrap().accepted, frames[..1]);
    assert_eq!(fixture.memory.lock().unwrap().read_limits.len(), 1);
    fixture
        .controller
        .dispatch(PlaybackCommand::Play(fixture.track.id))
        .unwrap();
    fixture
        .stream
        .play(&lease, fixture.play_operation())
        .unwrap();
    assert!(fixture.stream.poll().is_none()); // Previous Paused cannot acknowledge Play.
    fixture.acknowledge(&lease, fixture.play_operation(), OutputState::Running);
    assert!(matches!(
        fixture.stream.poll(),
        Some(BackendEvent::Started { .. })
    ));
    assert!(fixture.stream.poll().is_none());
    assert_eq!(fixture.memory.lock().unwrap().accepted, frames);
    assert_eq!(fixture.memory.lock().unwrap().output_rates, vec![44_100]);
}

#[test]
fn decoder_eof_and_accepted_pcm_do_not_end_until_output_really_drains() {
    let frames = [[0.15, -0.75], [-0.25, 0.5]];
    let mut fixture = Fixture::new(vec![block(&frames), DecodedRead::End]);
    let lease = fixture.start();
    assert!(fixture.stream.poll().is_none()); // PCM accepted by output.
    assert!(fixture.stream.poll().is_none()); // Decoder EOF; output still owns PCM.
    for _ in 0..3 {
        assert!(fixture.stream.poll().is_none());
    }
    assert_eq!(fixture.memory.lock().unwrap().accepted, frames);
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Playing);
    fixture.memory.lock().unwrap().drained = true;
    let ended = fixture.stream.poll().unwrap();
    assert!(
        matches!(ended, BackendEvent::Ended { generation } if generation == lease.generation())
    );
    fixture.deliver(ended);
    assert_eq!(fixture.controller.snapshot().state, PlaybackState::Stopped);
    assert_eq!(fixture.memory.lock().unwrap().clears, 1);
    assert_eq!(fixture.memory.lock().unwrap().output_releases, 1);
    assert_eq!(fixture.memory.lock().unwrap().decoder_releases, 1);
    assert!(fixture.stream.poll().is_none());
}

#[test]
fn cancellation_clears_pending_pcm_and_late_acknowledgments_cannot_restart_it() {
    let mut fixture = Fixture::new(vec![block(&[[0.1, 0.2], [0.3, 0.4]]), DecodedRead::End]);
    let lease = fixture.start();
    fixture
        .memory
        .lock()
        .unwrap()
        .writes
        .push_back(Ok(OutputWrite::Pending));
    assert!(fixture.stream.poll().is_none());
    let operation = fixture.play_operation();
    fixture.controller.dispatch(PlaybackCommand::Stop).unwrap();
    assert!(lease.is_cancelled());
    assert!(fixture.stream.poll().is_none()); // Revocation is sufficient before release arrives.
    fixture.stream.release(&lease);
    fixture.stream.play(&lease, operation).unwrap();
    fixture.acknowledge(&lease, operation, OutputState::Running);
    assert!(fixture.stream.poll().is_none());
    assert_eq!(fixture.stream.buffered_frames(), 0);
    let memory = fixture.memory.lock().unwrap();
    assert!(memory.accepted.is_empty());
    assert_eq!(memory.read_limits.len(), 1);
    assert_eq!(memory.clears, 1);
    assert_eq!(memory.output_releases, 1);
    assert_eq!(memory.decoder_releases, 1);
}

#[test]
fn releasing_an_old_generation_does_not_destroy_the_new_stream() {
    let mut fixture = Fixture::new(vec![block(&[[0.1, 0.2]])]);
    let old = fixture.start();
    fixture.controller.dispatch(PlaybackCommand::Stop).unwrap();
    fixture.stream.release(&old);
    fixture
        .memory
        .lock()
        .unwrap()
        .reads
        .push_back(Ok(block(&[[0.7, -0.9]])));
    let current = fixture.load();
    assert_ne!(old.generation(), current.generation());
    fixture.stream.release(&old);
    assert!(
        matches!(fixture.stream.poll(), Some(BackendEvent::Ready { generation })
        if generation == current.generation())
    );
    assert_eq!(fixture.stream.buffered_frames(), 1);
}

#[test]
fn superseded_control_operations_do_not_write_or_relabel_acknowledgments() {
    let mut fixture = Fixture::new(vec![block(&[[0.1, -0.2], [0.3, -0.4]])]);
    let lease = fixture.start();
    let old_play = fixture.play_operation();
    let pause = fixture.pause(&lease);
    fixture.stream.play(&lease, old_play).unwrap();
    fixture.acknowledge(&lease, old_play, OutputState::Running);
    assert!(fixture.stream.poll().is_none());
    assert!(fixture.memory.lock().unwrap().accepted.is_empty());
    fixture.acknowledge(&lease, pause, OutputState::Paused);
    let paused = fixture.stream.poll().unwrap();
    fixture.deliver(paused);
    fixture
        .controller
        .dispatch(PlaybackCommand::Play(fixture.track.id))
        .unwrap();
    let current = fixture.play_operation();
    fixture.stream.play(&lease, current).unwrap();
    fixture.stream.pause(&lease, pause).unwrap();
    fixture.acknowledge(&lease, old_play, OutputState::Running);
    assert!(fixture.stream.poll().is_none()); // Same state, stale operation.
    assert!(fixture.memory.lock().unwrap().accepted.is_empty());
    fixture.acknowledge(&lease, current, OutputState::Running);
    assert!(
        matches!(fixture.stream.poll(), Some(BackendEvent::Started { operation, .. })
        if operation == current)
    );
    assert!(fixture.stream.poll().is_none());
    assert_eq!(
        fixture.memory.lock().unwrap().accepted,
        vec![[0.1, -0.2], [0.3, -0.4]]
    );
}

#[test]
fn invalid_initial_pcm_and_empty_source_fail_without_ready_or_output() {
    let bad_reads = vec![
        DecodedRead::End,
        block(&[]),
        block(&[[f32::NAN, 0.0]]),
        block(&[[0.0, f32::INFINITY]]),
        block(&vec![[0.1, -0.1]; MAX_BUFFER_FRAMES + 1]),
        DecodedRead::Frames {
            sample_rate: 0,
            frames: vec![[0.1, -0.1]],
        },
    ];
    for read in bad_reads {
        let mut fixture = Fixture::new(vec![read]);
        let lease = fixture.load();
        fixture.failure(lease.generation());
        let memory = fixture.memory.lock().unwrap();
        assert!(memory.output_rates.is_empty());
        assert!(memory.accepted.is_empty());
        assert_eq!(memory.decoder_releases, 1);
    }
}

#[test]
fn rate_change_is_terminal_and_never_reinterprets_the_second_block() {
    let first = [[0.1, -0.3]];
    let mut fixture = Fixture::new(vec![
        block(&first),
        DecodedRead::Frames {
            sample_rate: 48_000,
            frames: vec![[0.6, -0.8]],
        },
    ]);
    let lease = fixture.start();
    assert!(fixture.stream.poll().is_none());
    fixture.failure(lease.generation());
    let memory = fixture.memory.lock().unwrap();
    assert_eq!(memory.accepted, first);
    assert_eq!(memory.output_rates, vec![44_100]);
    assert_eq!(memory.clears, 1);
    assert_eq!(memory.output_releases, 1);
}

#[test]
fn zero_and_overreported_writes_are_failures_while_pending_is_backpressure() {
    for count in [0, 3] {
        let mut fixture = Fixture::new(vec![block(&[[0.1, -0.1], [0.2, -0.2]])]);
        let lease = fixture.start();
        fixture
            .memory
            .lock()
            .unwrap()
            .writes
            .push_back(Ok(OutputWrite::Written(count)));
        fixture.failure(lease.generation());
        let memory = fixture.memory.lock().unwrap();
        assert!(memory.accepted.is_empty());
        assert_eq!(memory.clears, 1);
        assert_eq!(memory.output_releases, 1);
    }
}

#[test]
fn decoder_factory_and_incremental_read_errors_are_terminal() {
    for fault in ["decoder begin", "decode"] {
        let mut fixture = Fixture::new(vec![block(&[[0.1, 0.2]])]);
        fixture.memory.lock().unwrap().fault = Some(fault);
        let lease = fixture.load();
        fixture.failure(lease.generation());
        assert!(fixture.memory.lock().unwrap().accepted.is_empty());
    }
}

#[test]
fn output_creation_start_state_and_write_errors_fail_without_false_completion() {
    for fault in ["output begin", "start", "state", "write"] {
        let mut fixture = Fixture::new(vec![block(&[[0.1, 0.2]])]);
        let lease = fixture.load();
        let ready = fixture.stream.poll().unwrap();
        fixture.deliver(ready);
        fixture.memory.lock().unwrap().fault = Some(fault);
        fixture
            .stream
            .play(&lease, fixture.play_operation())
            .unwrap();
        fixture.acknowledge(&lease, fixture.play_operation(), OutputState::Running);
        if fault == "write" {
            let started = fixture.stream.poll().unwrap();
            assert!(matches!(started, BackendEvent::Started { .. }));
            fixture.deliver(started);
        }
        fixture.failure(lease.generation());
        assert!(fixture.memory.lock().unwrap().accepted.is_empty());
    }
}

#[test]
fn a_full_bounded_block_is_retained_during_backpressure_and_pause() {
    let frames: Vec<[f32; 2]> = (0..MAX_BUFFER_FRAMES)
        .map(|index| [index as f32 / MAX_BUFFER_FRAMES as f32, -0.25])
        .collect();
    let mut fixture = Fixture::new(vec![block(&frames), block(&[[0.75, -0.5]])]);
    let lease = fixture.start();
    fixture
        .memory
        .lock()
        .unwrap()
        .writes
        .push_back(Ok(OutputWrite::Pending));
    assert!(fixture.stream.poll().is_none());
    assert_eq!(fixture.stream.buffered_frames(), MAX_BUFFER_FRAMES);
    let pause = fixture.pause(&lease);
    fixture.acknowledge(&lease, pause, OutputState::Paused);
    let paused = fixture.stream.poll().unwrap();
    fixture.deliver(paused);
    for _ in 0..3 {
        assert!(fixture.stream.poll().is_none());
    }
    assert_eq!(
        fixture.memory.lock().unwrap().read_limits,
        vec![MAX_BUFFER_FRAMES]
    );
    assert!(fixture.memory.lock().unwrap().accepted.is_empty());
    fixture
        .controller
        .dispatch(PlaybackCommand::Play(fixture.track.id))
        .unwrap();
    fixture
        .stream
        .play(&lease, fixture.play_operation())
        .unwrap();
    fixture.acknowledge(&lease, fixture.play_operation(), OutputState::Running);
    assert!(matches!(
        fixture.stream.poll(),
        Some(BackendEvent::Started { .. })
    ));
    assert!(fixture.stream.poll().is_none());
    assert_eq!(fixture.memory.lock().unwrap().accepted, frames);
    assert_eq!(fixture.stream.buffered_frames(), 0);
    assert_eq!(
        fixture.memory.lock().unwrap().read_limits,
        vec![MAX_BUFFER_FRAMES]
    );
}

#[test]
fn cancellation_discards_a_queued_failure_and_drop_releases_owned_resources() {
    let mut fixture = Fixture::new(vec![]);
    fixture.memory.lock().unwrap().fault = Some("decoder begin");
    let lease = fixture.load();
    fixture.controller.dispatch(PlaybackCommand::Stop).unwrap();
    assert!(lease.is_cancelled());
    assert!(fixture.stream.poll().is_none());

    let mut fixture = Fixture::new(vec![block(&[[0.3, -0.4]])]);
    fixture.start();
    let memory = fixture.memory.clone();
    drop(fixture);
    let memory = memory.lock().unwrap();
    assert!(memory.accepted.is_empty());
    assert_eq!(memory.clears, 1);
    assert_eq!(memory.output_releases, 1);
    assert_eq!(memory.decoder_releases, 1);
}

#[test]
fn pause_and_drain_errors_release_the_session_without_false_completion() {
    let mut fixture = Fixture::new(vec![block(&[[0.1, -0.2]]), DecodedRead::End]);
    let lease = fixture.start();
    fixture.memory.lock().unwrap().fault = Some("pause");
    fixture.pause(&lease);
    fixture.failure(lease.generation());

    let mut fixture = Fixture::new(vec![block(&[[0.1, -0.2]]), DecodedRead::End]);
    let lease = fixture.start();
    assert!(fixture.stream.poll().is_none());
    fixture.memory.lock().unwrap().fault = Some("drain");
    fixture.failure(lease.generation());
    assert_eq!(fixture.memory.lock().unwrap().accepted, vec![[0.1, -0.2]]);
}
