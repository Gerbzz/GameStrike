#![cfg(all(windows, feature = "native-music-host"))]
use gamestrike::music::{
    Track,
    host::{
        buffer::stereo_ring,
        output::{
            CallbackTimes, CompletionCapability, DeviceCallback, DeviceDriver, ErrorSignal,
            OutputHandle, OwnerBudget, StreamControl, begin_with_driver, supports_exact_stereo_f32,
        },
    },
    playback::{BackendEvent, PlaybackBackend, PlaybackCommand, PlaybackController, PlaybackLease},
    streaming::{Output, OutputState, OutputWrite},
};
use std::{
    collections::VecDeque,
    marker::PhantomData,
    rc::Rc,
    sync::{Arc, Condvar, Mutex, atomic::{AtomicBool, Ordering}},
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Default)]
struct LeaseState {
    lease: Option<PlaybackLease>,
    operation: u64,
    events: VecDeque<BackendEvent>,
}
struct LeaseBackend(Arc<Mutex<LeaseState>>);
impl PlaybackBackend for LeaseBackend {
    fn load(&mut self, _: &Track, lease: PlaybackLease) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        state.events.push_back(BackendEvent::Ready { generation: lease.generation() });
        state.lease = Some(lease);
        Ok(())
    }
    fn play(&mut self, lease: &PlaybackLease, operation: u64) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        state.operation = operation;
        state.events.push_back(BackendEvent::Started { generation: lease.generation(), operation });
        Ok(())
    }
    fn pause(&mut self, lease: &PlaybackLease, operation: u64) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        state.operation = operation;
        state.events.push_back(BackendEvent::Paused { generation: lease.generation(), operation });
        Ok(())
    }
    fn release(&mut self, _: &PlaybackLease) {}
    fn poll(&mut self) -> Option<BackendEvent> { self.0.lock().unwrap().events.pop_front() }
}

#[derive(Default)]
struct DeviceState {
    callback: Option<DeviceCallback>,
    error: Option<ErrorSignal>,
    opened: usize,
    play: usize,
    pause: usize,
    dropped: usize,
    reject_rate: bool,
    open_gate: Option<Arc<Gate>>,
    play_gate: Option<Arc<Gate>>,
    play_error: bool,
}
#[derive(Default)]
struct Gate {
    entered: AtomicBool,
    released: Mutex<bool>,
    changed: Condvar,
}
impl Gate {
    fn wait(&self) {
        self.entered.store(true, Ordering::Release);
        let guard = self.released.lock().unwrap();
        drop(self.changed.wait_while(guard, |released| !*released).unwrap());
    }
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
}
struct ReleaseOnDrop(Arc<Gate>);
impl Drop for ReleaseOnDrop { fn drop(&mut self) { self.0.release(); } }
struct FakeDevice(Arc<Mutex<DeviceState>>);
struct FakeStream {
    state: Arc<Mutex<DeviceState>>,
    // A non-Send stream must still work: it is born and dropped on the owner thread.
    _thread_only: PhantomData<Rc<()>>,
}
impl DeviceDriver for FakeDevice {
    type Stream = FakeStream;
    fn open(self, rate: u32, callback: DeviceCallback, error: ErrorSignal) -> Result<FakeStream, String> {
        let mut state = self.0.lock().unwrap();
        state.opened += 1;
        if state.reject_rate || rate != 48_000 { return Err("selected fixture device rejects rate".into()); }
        let gate = state.open_gate.take();
        drop(state);
        if let Some(gate) = gate { gate.wait(); }
        let mut state = self.0.lock().unwrap();
        state.callback = Some(callback);
        state.error = Some(error);
        drop(state);
        Ok(FakeStream { state: self.0, _thread_only: PhantomData })
    }
}
impl StreamControl for FakeStream {
    fn play(&self) -> Result<(), String> {
        let gate = { let mut state = self.state.lock().unwrap(); state.play += 1; state.play_gate.take() };
        if let Some(gate) = gate { gate.wait(); }
        if self.state.lock().unwrap().play_error { return Err("fixture output failed".into()); }
        Ok(())
    }
    fn pause(&self) -> Result<(), String> { self.state.lock().unwrap().pause += 1; Ok(()) }
}
impl Drop for FakeStream {
    fn drop(&mut self) { self.state.lock().unwrap().dropped += 1; }
}

struct Fixture {
    _temp: tempfile::TempDir,
    controller: PlaybackController,
    lease_state: Arc<Mutex<LeaseState>>,
    id: Uuid,
    lease: PlaybackLease,
    operation: u64,
    device: Arc<Mutex<DeviceState>>,
    output: OutputHandle,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let lease_state = Arc::new(Mutex::new(LeaseState::default()));
        let mut controller = PlaybackController::new(Box::new(LeaseBackend(lease_state.clone())));
        let id = Uuid::from_u128(1);
        controller.replace_queue(&[Track { id, title: "Owned fixture".into(), path: temp.path().join("owned.wav") }]).unwrap();
        controller.dispatch(PlaybackCommand::Play(id)).unwrap();
        controller.poll();
        let lease = lease_state.lock().unwrap().lease.clone().unwrap();
        let operation = lease_state.lock().unwrap().operation;
        let device = Arc::new(Mutex::new(DeviceState::default()));
        let output = begin_with_driver(
            FakeDevice(device.clone()), 48_000, lease.clone(), 4, OwnerBudget::new(2).unwrap(),
        ).unwrap();
        let fixture = Self { _temp: temp, controller, lease_state, id, lease, operation, device, output };
        wait(|| fixture.device.lock().unwrap().opened == 1);
        fixture
    }
    fn callback(&self, samples: &mut [f32], time: Option<CallbackTimes>) {
        let mut callback = self.device.lock().unwrap().callback.take().unwrap();
        callback.render(samples, time);
        self.device.lock().unwrap().callback = Some(callback);
    }
    fn start(&mut self) {
        self.output.start(&self.lease, self.operation).unwrap();
        wait(|| self.device.lock().unwrap().play > 0);
        wait(|| self.output.api_play_accepted(self.operation));
        self.callback(&mut [1.0; 4], Some(stamp(1_000_000, 2_000_000)));
        assert_eq!(self.output.poll_state(&self.lease, self.operation).unwrap(), OutputState::Running);
    }
}
fn wait(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !predicate() {
        assert!(Instant::now() < deadline, "owner/callback seam did not progress");
        std::thread::yield_now();
    }
}
fn stamp(callback_ns: u64, playback_ns: u64) -> CallbackTimes { CallbackTimes { callback_ns, playback_ns } }

#[test]
fn spsc_ring_preserves_stereo_order_and_applies_bounded_backpressure() {
    let (mut producer, mut consumer) = stereo_ring(2).unwrap();
    assert_eq!(producer.write(&[[0.1, 0.2], [0.3, 0.4], [0.5, 0.6]]).unwrap(), 2);
    assert_eq!(producer.write(&[[0.9, 1.0]]).unwrap(), 0);
    let mut data = [0.0; 2];
    let ticket = consumer.prepare(&mut data);
    assert_eq!(data, [0.1, 0.2]);
    assert_eq!(producer.queued(), 2, "preparation cannot free a slot before the authority check");
    assert!(consumer.commit(ticket));
    assert_eq!(producer.write(&[[0.5, 0.6]]).unwrap(), 1);
    let mut data = [0.0; 4];
    let ticket = consumer.prepare(&mut data);
    assert_eq!(data, [0.3, 0.4, 0.5, 0.6]);
    assert!(consumer.commit(ticket));
    assert_eq!(producer.queued(), 0);
    assert!(producer.write(&[[f32::NAN, 0.0]]).is_err());
    assert_eq!(producer.queued(), 0);
    assert!(stereo_ring(3).is_err());

    let (mut other_producer, mut other_consumer) = stereo_ring(2).unwrap();
    other_producer.write(&[[0.7, 0.8]]).unwrap();
    let foreign_ticket = other_consumer.prepare(&mut [0.0; 2]);
    let (mut empty_producer, mut empty_consumer) = stereo_ring(2).unwrap();
    assert!(!empty_consumer.commit(foreign_ticket));
    assert_eq!(empty_consumer.consumed(), 0);
    assert_eq!(empty_consumer.prepare(&mut [0.0; 2]).frames(), 0);
    assert_eq!(empty_producer.write(&[[0.9, 1.0]]).unwrap(), 1);
}

#[test]
fn exact_source_config_has_no_channel_format_or_rate_fallback() {
    assert!(supports_exact_stereo_f32(2, true, 44_100, 48_000, 48_000));
    assert!(!supports_exact_stereo_f32(1, true, 44_100, 48_000, 48_000));
    assert!(!supports_exact_stereo_f32(2, false, 44_100, 48_000, 48_000));
    assert!(!supports_exact_stereo_f32(2, true, 44_100, 44_100, 48_000));
}

#[test]
fn preparation_does_not_play_and_running_requires_a_current_callback_after_api_acceptance() {
    let mut fixture = Fixture::new();
    assert_eq!(fixture.device.lock().unwrap().play, 0);
    assert_eq!(fixture.output.poll_state(&fixture.lease, fixture.operation).unwrap(), OutputState::Pending);
    fixture.output.start(&fixture.lease, fixture.operation).unwrap();
    wait(|| fixture.output.api_play_accepted(fixture.operation));
    assert_eq!(fixture.output.poll_state(&fixture.lease, fixture.operation).unwrap(), OutputState::Pending);
    fixture.callback(&mut [9.0; 4], Some(stamp(1_000_000, 2_000_000)));
    assert_eq!(fixture.output.poll_state(&fixture.lease, fixture.operation).unwrap(), OutputState::Running);
}

#[test]
fn pause_gates_callbacks_and_preserves_pcm_until_a_new_current_start() {
    let mut fixture = Fixture::new();
    fixture.start();
    assert_eq!(fixture.output.try_write(&[[0.25, -0.25], [0.5, -0.5]]).unwrap(), OutputWrite::Written(2));
    fixture.controller.dispatch(PlaybackCommand::Pause).unwrap();
    fixture.controller.poll();
    let pause = fixture.lease_state.lock().unwrap().operation;
    fixture.output.pause(&fixture.lease, pause).unwrap();
    wait(|| fixture.output.poll_state(&fixture.lease, pause).unwrap() == OutputState::Paused);
    let mut zero = [9.0; 4];
    fixture.callback(&mut zero, Some(stamp(2_000_000, 3_000_000)));
    assert_eq!(zero, [0.0; 4]);
    fixture.controller.dispatch(PlaybackCommand::Play(fixture.id)).unwrap();
    fixture.controller.poll();
    let resume = fixture.lease_state.lock().unwrap().operation;
    fixture.output.start(&fixture.lease, resume).unwrap();
    assert_eq!(fixture.output.poll_state(&fixture.lease, resume).unwrap(), OutputState::Pending);
    wait(|| fixture.output.api_play_accepted(resume));
    let mut media = [0.0; 4];
    fixture.callback(&mut media, Some(stamp(3_000_000, 4_000_000)));
    assert_eq!(media, [0.25, -0.25, 0.5, -0.5]);
    assert_eq!(fixture.output.poll_state(&fixture.lease, resume).unwrap(), OutputState::Running);
}

#[test]
fn stale_operations_and_released_output_cannot_consume_or_acknowledge_pcm() {
    let mut fixture = Fixture::new();
    fixture.start();
    fixture.output.try_write(&[[0.25, 0.5]]).unwrap();
    fixture.controller.dispatch(PlaybackCommand::Stop).unwrap();
    assert_eq!(fixture.output.try_write(&[[0.75, 1.0]]).unwrap(), OutputWrite::Pending);
    fixture.output.start(&fixture.lease, fixture.operation).unwrap();
    let mut zero = [9.0; 2];
    fixture.callback(&mut zero, Some(stamp(2_000_000, 3_000_000)));
    assert_eq!(zero, [0.0; 2]);
    assert_eq!(fixture.output.poll_state(&fixture.lease, fixture.operation).unwrap(), OutputState::Pending);
    fixture.output.release();
    fixture.output.release();
    assert!(!fixture.output.drained().unwrap());
}

#[test]
fn genuine_current_device_errors_are_terminal_and_callback_layout_errors_zero_output() {
    let mut fixture = Fixture::new();
    fixture.start();
    let mut invalid = [9.0; 3];
    fixture.callback(&mut invalid, Some(stamp(2_000_000, 3_000_000)));
    assert_eq!(invalid, [0.0; 3]);
    assert!(fixture.output.poll_state(&fixture.lease, fixture.operation).is_err());
    let mut other = Fixture::new();
    other.start();
    other.device.lock().unwrap().error.as_ref().unwrap().fail();
    assert!(other.output.try_write(&[[0.0, 0.0]]).is_err());
    assert!(!other.output.prediction_complete());
}

#[test]
fn predicted_deadline_progress_never_satisfies_strict_real_output_drain() {
    let mut fixture = Fixture::new();
    fixture.start();
    fixture.output.try_write(&[[0.5, -0.5]]).unwrap();
    fixture.callback(&mut [0.0; 2], Some(stamp(2_000_000, 3_000_000)));
    assert!(!fixture.output.prediction_complete(), "empty ring is not completion");
    fixture.callback(&mut [0.0; 2], Some(stamp(4_000_000, 5_000_000)));
    assert!(fixture.output.prediction_complete());
    fixture.controller.dispatch(PlaybackCommand::Pause).unwrap();
    fixture.controller.poll();
    let pause = fixture.lease_state.lock().unwrap().operation;
    fixture.output.pause(&fixture.lease, pause).unwrap();
    wait(|| fixture.output.poll_state(&fixture.lease, pause).unwrap() == OutputState::Paused);
    fixture.controller.dispatch(PlaybackCommand::Play(fixture.id)).unwrap();
    fixture.controller.poll();
    let resume = fixture.lease_state.lock().unwrap().operation;
    assert!(!fixture.output.prediction_complete(), "old operation cannot predict resume completion");
    fixture.output.start(&fixture.lease, resume).unwrap();
    assert!(!fixture.output.prediction_complete());
    wait(|| fixture.output.api_play_accepted(resume));
    assert!(!fixture.output.prediction_complete(), "new API acceptance is not a new callback prediction");
    fixture.callback(&mut [0.0; 2], Some(stamp(6_000_000, 7_000_000)));
    assert!(!fixture.output.prediction_complete(), "resume establishes a fresh presentation boundary");
    fixture.callback(&mut [0.0; 2], Some(stamp(8_000_000, 9_000_000)));
    assert!(fixture.output.prediction_complete());
    assert_eq!(fixture.output.completion_capability(), CompletionCapability::PredictionOnly);
    assert!(!fixture.output.drained().unwrap(), "CPAL provides no hardware completion proof");
}

#[test]
fn unavailable_or_nonmonotonic_timing_fails_closed_for_completion_without_a_fake_device_failure() {
    for bad_time in [None, Some(stamp(0, 0))] {
        let mut fixture = Fixture::new();
        fixture.start();
        fixture.output.try_write(&[[0.5, -0.5]]).unwrap();
        fixture.callback(&mut [0.0; 2], bad_time);
        fixture.callback(&mut [0.0; 2], Some(stamp(9_000_000, 10_000_000)));
        assert!(!fixture.output.prediction_complete());
        assert!(!fixture.output.drained().unwrap());
        assert!(fixture.output.poll_state(&fixture.lease, fixture.operation).is_ok());
    }
}

#[test]
fn released_blocked_opens_retain_owner_permits_until_actual_worker_exit() {
    let fixture = Fixture::new();
    let budget = OwnerBudget::new(1).unwrap();
    let gate = Arc::new(Gate::default());
    let release = ReleaseOnDrop(gate.clone());
    let state = Arc::new(Mutex::new(DeviceState { open_gate: Some(gate.clone()), ..Default::default() }));
    let mut output = begin_with_driver(
        FakeDevice(state.clone()), 48_000, fixture.lease.clone(), 4, budget.clone(),
    ).unwrap();
    wait(|| gate.entered.load(Ordering::Acquire));
    output.release();
    assert_eq!(budget.active(), 1);
    assert!(begin_with_driver(
        FakeDevice(state), 48_000, fixture.lease.clone(), 4, budget.clone(),
    ).is_err());
    release.0.release();
    wait(|| budget.active() == 0);
}

#[test]
fn a_superseded_play_api_completion_cannot_acknowledge_a_newer_pause() {
    let mut fixture = Fixture::new();
    let gate = Arc::new(Gate::default());
    let release = ReleaseOnDrop(gate.clone());
    fixture.device.lock().unwrap().play_gate = Some(gate.clone());
    fixture.output.start(&fixture.lease, fixture.operation).unwrap();
    wait(|| gate.entered.load(Ordering::Acquire));
    fixture.controller.dispatch(PlaybackCommand::Pause).unwrap();
    fixture.controller.poll();
    let pause = fixture.lease_state.lock().unwrap().operation;
    fixture.output.pause(&fixture.lease, pause).unwrap();
    let mut zero = [9.0; 2];
    fixture.callback(&mut zero, Some(stamp(1_000_000, 2_000_000)));
    assert_eq!(zero, [0.0; 2]);
    assert_eq!(fixture.output.poll_state(&fixture.lease, pause).unwrap(), OutputState::Pending);
    release.0.release();
    wait(|| fixture.output.poll_state(&fixture.lease, pause).unwrap() == OutputState::Paused);
    assert!(!fixture.output.api_play_accepted(fixture.operation));
}

#[test]
fn selected_device_rejection_is_terminal_without_another_config_or_play_attempt() {
    let fixture = Fixture::new();
    let state = Arc::new(Mutex::new(DeviceState::default()));
    let mut output = begin_with_driver(
        FakeDevice(state.clone()), 44_100, fixture.lease.clone(), 4, OwnerBudget::new(1).unwrap(),
    ).unwrap();
    wait(|| output.poll_state(&fixture.lease, fixture.operation).is_err());
    assert_eq!(state.lock().unwrap().opened, 1);
    assert_eq!(state.lock().unwrap().play, 0);
}

#[test]
fn a_real_device_failure_remains_terminal_after_a_newer_opposite_request() {
    let mut fixture = Fixture::new();
    let gate = Arc::new(Gate::default());
    let release = ReleaseOnDrop(gate.clone());
    {
        let mut state = fixture.device.lock().unwrap();
        state.play_gate = Some(gate.clone());
        state.play_error = true;
    }
    fixture.output.start(&fixture.lease, fixture.operation).unwrap();
    wait(|| gate.entered.load(Ordering::Acquire));
    fixture.controller.dispatch(PlaybackCommand::Pause).unwrap();
    fixture.controller.poll();
    let pause = fixture.lease_state.lock().unwrap().operation;
    fixture.output.pause(&fixture.lease, pause).unwrap();
    release.0.release();
    let mut first_error = None;
    wait(|| match fixture.output.poll_state(&fixture.lease, pause) {
        Err(error) => { first_error = Some(error); true }
        Ok(_) => false,
    });
    assert!(first_error.unwrap().contains("fixture output failed"));
    assert_eq!(fixture.device.lock().unwrap().pause, 0);
}
