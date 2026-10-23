//! Explicit-device Windows CPAL output; construction opens no device.
//! CPAL provides predicted presentation times, not physical completion. Strict
//! `Output::drained` therefore stays false; natural EOF/advance is not implemented.
use super::buffer::{Consumer, MAX_RING_FRAMES, Producer, stereo_ring};
use crate::music::{
    playback::PlaybackLease,
    streaming::{Output, OutputFactory, OutputState, OutputWrite},
};
use std::{
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::Duration,
};

const RUNNING: u8 = 1;
const PAUSED: u8 = 2;
const CALLBACK_ERROR: u8 = 1;
const PROTOCOL_ERROR: u8 = 2;
const OWNER_ERROR: u8 = 3;
const MAX_CALLBACK_FRAMES: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionCapability {
    PredictionOnly,
}

#[derive(Clone, Copy, Debug)]
pub struct CallbackTimes {
    pub callback_ns: u64,
    pub playback_ns: u64,
}

struct Budget {
    limit: usize,
    active: AtomicUsize,
}
#[derive(Clone)]
pub struct OwnerBudget(Arc<Budget>);
impl OwnerBudget {
    pub fn new(limit: usize) -> Result<Self, String> {
        if !(1..=2).contains(&limit) {
            return Err("Output owner budget must be 1 or 2".into());
        }
        Ok(Self(Arc::new(Budget {
            limit,
            active: AtomicUsize::new(0),
        })))
    }
    fn global() -> Self {
        static BUDGET: OnceLock<OwnerBudget> = OnceLock::new();
        BUDGET
            .get_or_init(|| Self::new(2).expect("constant owner budget"))
            .clone()
    }
    pub fn active(&self) -> usize {
        self.0.active.load(Ordering::Acquire)
    }
    fn acquire(&self) -> Result<OwnerPermit, String> {
        self.0
            .active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < self.0.limit).then_some(active + 1)
            })
            .map_err(|_| {
                "Output owner capacity is occupied; wait for prior device release".to_owned()
            })?;
        Ok(OwnerPermit(self.clone()))
    }
}
struct OwnerPermit(OwnerBudget);
impl Drop for OwnerPermit {
    fn drop(&mut self) {
        self.0.0.active.fetch_sub(1, Ordering::AcqRel);
    }
}

struct Shared {
    lease: PlaybackLease,
    alive: AtomicBool,
    finished: AtomicBool,
    requested: AtomicU8,
    operation: AtomicU64,
    api_play: AtomicU64,
    api_pause: AtomicU64,
    ack_state: AtomicU8,
    ack_operation: AtomicU64,
    callbacks: AtomicUsize,
    error: AtomicU8,
    predicted: AtomicBool,
    predicted_written: AtomicU64,
    predicted_operation: AtomicU64,
}
impl Shared {
    fn request(&self) -> Option<(u64, u8)> {
        let operation = self.operation.load(Ordering::Acquire);
        let state = self.requested.load(Ordering::Acquire);
        if operation == 0
            || !self.alive.load(Ordering::Acquire)
            || !self.lease.is_current(operation)
            || (state == RUNNING) != self.lease.may_output()
            || !matches!(state, RUNNING | PAUSED)
        {
            return None;
        }
        Some((operation, state))
    }
    fn running(&self, operation: u64) -> bool {
        self.error.load(Ordering::Acquire) == 0
            && self.request() == Some((operation, RUNNING))
            && self.api_play.load(Ordering::Acquire) == operation
    }
    fn acknowledge(&self, operation: u64, state: u8) {
        if self.request() == Some((operation, state)) {
            self.ack_state.store(state, Ordering::Relaxed);
            self.ack_operation.store(operation, Ordering::Release);
        }
    }
}

/// No allocation, lock, channel or format operation occurs in this callback path.
pub struct DeviceCallback {
    consumer: Consumer,
    shared: Arc<Shared>,
    rate: u32,
    timing_valid: bool,
    previous_callback: Option<u64>,
    deadline: Option<u64>,
    previous_operation: u64,
}
struct InCallback<'a>(&'a AtomicUsize);
impl Drop for InCallback<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl DeviceCallback {
    pub fn render(&mut self, samples: &mut [f32], times: Option<CallbackTimes>) {
        self.shared.callbacks.fetch_add(1, Ordering::SeqCst);
        let _callback = InCallback(&self.shared.callbacks);
        samples.fill(0.0);
        if !samples.len().is_multiple_of(2) || samples.len() / 2 > MAX_CALLBACK_FRAMES {
            self.shared.error.store(PROTOCOL_ERROR, Ordering::Release);
            return;
        }
        let Some((operation, RUNNING)) = self.shared.request() else {
            return;
        };
        if !self.shared.running(operation) {
            return;
        }
        let ticket = self.consumer.prepare(samples);
        let media = ticket.frames();
        if samples.iter().any(|sample| !sample.is_finite()) {
            samples.fill(0.0);
            self.shared.error.store(PROTOCOL_ERROR, Ordering::Release);
            return;
        }
        // The producer cannot reuse prepared slots until this final authority
        // check and commit. A paused/stale callback loses no queued PCM.
        if !self.shared.running(operation) || !self.consumer.commit(ticket) {
            samples.fill(0.0);
            return;
        }
        self.shared.acknowledge(operation, RUNNING);
        self.shared.predicted.store(false, Ordering::Release);
        if let Some(times) = times {
            if times.playback_ns < times.callback_ns
                || self
                    .previous_callback
                    .is_some_and(|previous| times.callback_ns <= previous)
            {
                self.timing_valid = false;
            }
            self.previous_callback = Some(times.callback_ns);
            if self.timing_valid {
                let duration = |frames: usize| -> Option<u64> {
                    let value = (frames as u128 * 1_000_000_000).div_ceil(self.rate as u128);
                    value.try_into().ok()
                };
                // Resume must not reuse a pre-pause deadline for hardware PCM
                // that may have remained buffered while the stream was stopped.
                let boundary = if operation != self.previous_operation {
                    duration(samples.len() / 2)
                        .and_then(|duration| times.playback_ns.checked_add(duration))
                } else {
                    None
                };
                let media_deadline = if media > 0 {
                    duration(media).and_then(|duration| times.playback_ns.checked_add(duration))
                } else {
                    None
                };
                for deadline in [boundary, media_deadline].into_iter().flatten() {
                    self.deadline = Some(
                        self.deadline
                            .map_or(deadline, |previous| previous.max(deadline)),
                    );
                }
                if (media > 0 && media_deadline.is_none())
                    || (operation != self.previous_operation && boundary.is_none())
                {
                    self.timing_valid = false;
                }
                if media == 0
                    && self.consumer.written() == self.consumer.consumed()
                    && self
                        .deadline
                        .is_some_and(|deadline| times.callback_ns > deadline)
                    && self.timing_valid
                    && self.shared.running(operation)
                {
                    self.shared
                        .predicted_written
                        .store(self.consumer.written(), Ordering::Relaxed);
                    self.shared
                        .predicted_operation
                        .store(operation, Ordering::Relaxed);
                    self.shared.predicted.store(true, Ordering::Release);
                }
            }
        } else {
            self.timing_valid = false;
        }
        self.previous_operation = operation;
    }
}

#[derive(Clone)]
pub struct ErrorSignal(Arc<Shared>);
impl ErrorSignal {
    pub fn fail(&self) {
        if self.0.alive.load(Ordering::Acquire) && !self.0.lease.is_cancelled() {
            self.0.error.store(CALLBACK_ERROR, Ordering::Release);
        }
    }
}

/// The associated stream need not be Send. It is constructed and destroyed only
/// inside the owner worker. The real implementation below uses CPAL/WASAPI.
pub trait DeviceDriver: Send + 'static {
    type Stream: StreamControl;
    fn open(
        self,
        sample_rate: u32,
        callback: DeviceCallback,
        errors: ErrorSignal,
    ) -> Result<Self::Stream, String>;
}
pub trait StreamControl {
    fn play(&self) -> Result<(), String>;
    fn pause(&self) -> Result<(), String>;
}

pub struct OutputHandle {
    producer: Producer,
    shared: Arc<Shared>,
    wake: SyncSender<()>,
    replies: Receiver<String>,
    problem: Option<String>,
    released: bool,
}
impl OutputHandle {
    pub fn completion_capability(&self) -> CompletionCapability {
        CompletionCapability::PredictionOnly
    }
    pub fn api_play_accepted(&self, operation: u64) -> bool {
        self.shared.api_play.load(Ordering::Acquire) == operation && self.shared.running(operation)
    }
    /// Diagnostic prediction only. Never satisfies `Output::drained`.
    pub fn prediction_complete(&self) -> bool {
        let Some((operation, RUNNING)) = self.shared.request() else {
            return false;
        };
        !self.released
            && self.shared.alive.load(Ordering::Acquire)
            && !self.shared.finished.load(Ordering::Acquire)
            && self.shared.error.load(Ordering::Acquire) == 0
            && self.shared.lease.may_output()
            && self.shared.predicted.load(Ordering::Acquire)
            && self.shared.predicted_operation.load(Ordering::Acquire) == operation
            && self.shared.predicted_written.load(Ordering::Acquire) == self.producer.written()
            && self.producer.queued() == 0
            && self.shared.running(operation)
    }
    fn failure(&mut self) -> Option<String> {
        while let Ok(problem) = self.replies.try_recv() {
            self.problem = Some(problem);
        }
        if self.released || self.shared.lease.is_cancelled() {
            return None;
        }
        let mut error = self.shared.error.load(Ordering::Acquire);
        let finished = self.shared.finished.load(Ordering::Acquire);
        if error == 0 && finished {
            error = self.shared.error.load(Ordering::Acquire);
        }
        if error == OWNER_ERROR {
            // The detail send precedes OWNER_ERROR's Release store. Re-read after
            // acquiring that publication, so the FIRST terminal result keeps it.
            while let Ok(problem) = self.replies.try_recv() {
                self.problem = Some(problem);
            }
        }
        match error {
            0 if !finished => None,
            CALLBACK_ERROR => Some("Selected output device reported a stream failure".into()),
            PROTOCOL_ERROR => Some("Invalid stereo output callback or PCM".into()),
            _ => Some(
                self.problem
                    .clone()
                    .unwrap_or_else(|| "Selected output owner is unavailable".into()),
            ),
        }
    }
    fn request(&mut self, lease: &PlaybackLease, operation: u64, state: u8) -> Result<(), String> {
        if self.released
            || lease.generation() != self.shared.lease.generation()
            || !lease.is_current(operation)
            || !self.shared.lease.is_current(operation)
            || (state == RUNNING) != lease.may_output()
        {
            return Ok(());
        }
        if let Some(error) = self.failure() {
            return Err(error);
        }
        self.shared.predicted.store(false, Ordering::Release);
        self.shared.requested.store(state, Ordering::Relaxed);
        self.shared.operation.store(operation, Ordering::Release);
        let _ = self.wake.try_send(());
        Ok(())
    }
}
impl Output for OutputHandle {
    fn start(&mut self, lease: &PlaybackLease, operation: u64) -> Result<(), String> {
        self.request(lease, operation, RUNNING)
    }
    fn pause(&mut self, lease: &PlaybackLease, operation: u64) -> Result<(), String> {
        self.request(lease, operation, PAUSED)
    }
    fn poll_state(&mut self, lease: &PlaybackLease, operation: u64) -> Result<OutputState, String> {
        if self.released
            || !lease.is_current(operation)
            || !self.shared.lease.is_current(operation)
            || lease.generation() != self.shared.lease.generation()
        {
            return Ok(OutputState::Pending);
        }
        if let Some(error) = self.failure() {
            return Err(error);
        }
        if self.shared.ack_operation.load(Ordering::Acquire) != operation {
            return Ok(OutputState::Pending);
        }
        Ok(match self.shared.ack_state.load(Ordering::Acquire) {
            RUNNING if self.shared.request() == Some((operation, RUNNING)) => OutputState::Running,
            PAUSED if self.shared.request() == Some((operation, PAUSED)) => OutputState::Paused,
            _ => OutputState::Pending,
        })
    }
    fn try_write(&mut self, frames: &[[f32; 2]]) -> Result<OutputWrite, String> {
        if self.released || self.shared.lease.is_cancelled() {
            return Ok(OutputWrite::Pending);
        }
        if let Some(error) = self.failure() {
            return Err(error);
        }
        let Some((operation, RUNNING)) = self.shared.request() else {
            return Ok(OutputWrite::Pending);
        };
        if !self.shared.running(operation)
            || self.shared.ack_operation.load(Ordering::Acquire) != operation
            || self.shared.ack_state.load(Ordering::Acquire) != RUNNING
        {
            return Ok(OutputWrite::Pending);
        }
        let count = self.producer.write(frames)?;
        Ok(if count == 0 {
            OutputWrite::Pending
        } else {
            OutputWrite::Written(count)
        })
    }
    fn drained(&mut self) -> Result<bool, String> {
        if let Some(error) = self.failure() {
            return Err(error);
        }
        // No CPAL API certifies all device/hardware samples consumed. Predicted
        // timestamps, elapsed wall time and ring emptiness are insufficient.
        Ok(false)
    }
    fn clear(&mut self) {
        self.released = true;
        self.shared.alive.store(false, Ordering::Release);
        self.shared.predicted.store(false, Ordering::Release);
        let _ = self.wake.try_send(());
    }
    fn release(&mut self) {
        self.clear();
    }
}
impl Drop for OutputHandle {
    fn drop(&mut self) {
        self.clear();
    }
}

pub fn begin_with_driver<D: DeviceDriver>(
    driver: D,
    sample_rate: u32,
    lease: PlaybackLease,
    capacity: usize,
    budget: OwnerBudget,
) -> Result<OutputHandle, String> {
    if sample_rate == 0 || lease.is_cancelled() {
        return Err("Output source rate or authority is unavailable".into());
    }
    let permit = budget.acquire()?;
    let (producer, consumer) = stereo_ring(capacity)?;
    let shared = Arc::new(Shared {
        lease,
        alive: AtomicBool::new(true),
        finished: AtomicBool::new(false),
        requested: AtomicU8::new(0),
        operation: AtomicU64::new(0),
        api_play: AtomicU64::new(0),
        api_pause: AtomicU64::new(0),
        ack_state: AtomicU8::new(0),
        ack_operation: AtomicU64::new(0),
        callbacks: AtomicUsize::new(0),
        error: AtomicU8::new(0),
        predicted: AtomicBool::new(false),
        predicted_written: AtomicU64::new(0),
        predicted_operation: AtomicU64::new(0),
    });
    let (wake, commands) = mpsc::sync_channel(1);
    let (reply, replies) = mpsc::sync_channel(1);
    let owner = shared.clone();
    let worker = thread::Builder::new()
        .name("gamestrike-selected-output".into())
        .spawn(move || {
            own(
                driver,
                consumer,
                owner,
                sample_rate,
                commands,
                reply,
                permit,
            )
        })
        .map_err(|error| format!("Could not start bounded output owner: {error}"))?;
    drop(worker);
    Ok(OutputHandle {
        producer,
        shared,
        wake,
        replies,
        problem: None,
        released: false,
    })
}

struct OwnerExit(Arc<Shared>);
impl Drop for OwnerExit {
    fn drop(&mut self) {
        self.0.alive.store(false, Ordering::Release);
        self.0.finished.store(true, Ordering::Release);
    }
}
fn owner_failure(shared: &Shared, reply: &SyncSender<String>, error: String) {
    if shared.lease.is_cancelled() || !shared.alive.load(Ordering::Acquire) {
        return;
    }
    let error: String = error
        .chars()
        .filter(|character| !character.is_control())
        .take(512)
        .collect();
    let _ = reply.try_send(error);
    shared.error.store(OWNER_ERROR, Ordering::Release);
}
fn own<D: DeviceDriver>(
    driver: D,
    consumer: Consumer,
    shared: Arc<Shared>,
    rate: u32,
    commands: Receiver<()>,
    reply: SyncSender<String>,
    _permit: OwnerPermit,
) {
    let _exit = OwnerExit(shared.clone());
    if shared.lease.is_cancelled() || !shared.alive.load(Ordering::Acquire) {
        return;
    }
    let callback = DeviceCallback {
        consumer,
        shared: shared.clone(),
        rate,
        timing_valid: true,
        previous_callback: None,
        deadline: None,
        previous_operation: 0,
    };
    let stream = match driver.open(rate, callback, ErrorSignal(shared.clone())) {
        Ok(stream) => stream,
        Err(error) => {
            owner_failure(&shared, &reply, error);
            return;
        }
    };
    let mut applied = None;
    while shared.alive.load(Ordering::Acquire) && !shared.lease.is_cancelled() {
        if shared.error.load(Ordering::Acquire) != 0 {
            return;
        }
        if let Some(request @ (operation, state)) = shared.request() {
            if applied != Some(request) {
                // Recheck immediately before the selected-device call and again
                // before publishing. A later request cannot inherit this result.
                if shared.request() != Some(request) {
                    continue;
                }
                let result = if state == RUNNING {
                    stream.play()
                } else {
                    stream.pause()
                };
                // A real device failure belongs to the surviving session even
                // if a newer opposite operation superseded this call meanwhile.
                if let Err(error) = result {
                    owner_failure(&shared, &reply, error);
                    return;
                }
                if shared.request() == Some(request) {
                    if state == RUNNING {
                        shared.api_play.store(operation, Ordering::Release);
                    } else {
                        shared.api_pause.store(operation, Ordering::Release);
                    }
                }
                applied = Some(request);
            }
            // CPAL pause() Ok only enqueues WASAPI work. This acknowledges OUR
            // gate closed and callback-consumer quiescence, not hardware pause.
            if state == PAUSED
                && shared.api_pause.load(Ordering::Acquire) == operation
                && shared.callbacks.load(Ordering::SeqCst) == 0
            {
                shared.acknowledge(operation, PAUSED);
            }
        }
        let _ = commands.recv_timeout(Duration::from_millis(5));
    }
    // CPAL may join its internal callback thread on Drop. This remains on this
    // owner worker, and its global permit is held until actual destruction exits.
    drop(stream);
}

pub fn supports_exact_stereo_f32(
    channels: u16,
    f32_format: bool,
    min: u32,
    max: u32,
    rate: u32,
) -> bool {
    channels == 2 && f32_format && rate != 0 && min <= rate && rate <= max
}

#[cfg(windows)]
mod cpal_driver {
    use super::*;
    use cpal::traits::{DeviceTrait, StreamTrait};

    /// Holds exactly the caller-selected device. No discovery, default lookup,
    /// config query, stream creation or playback happens in these constructors.
    pub struct CpalOutputFactory {
        selected: cpal::Device,
        capacity: usize,
    }
    impl CpalOutputFactory {
        pub fn selected(device: cpal::Device) -> Self {
            Self {
                selected: device,
                capacity: MAX_RING_FRAMES,
            }
        }
        pub fn with_capacity(device: cpal::Device, capacity: usize) -> Result<Self, String> {
            if capacity == 0 || capacity > MAX_RING_FRAMES || !capacity.is_power_of_two() {
                return Err("Invalid bounded PCM capacity".into());
            }
            Ok(Self {
                selected: device,
                capacity,
            })
        }
        pub fn completion_capability(&self) -> CompletionCapability {
            CompletionCapability::PredictionOnly
        }
    }
    impl OutputFactory for CpalOutputFactory {
        fn begin(
            &mut self,
            sample_rate: u32,
            lease: PlaybackLease,
        ) -> Result<Box<dyn Output>, String> {
            Ok(Box::new(begin_with_driver(
                CpalDevice(self.selected.clone()),
                sample_rate,
                lease,
                self.capacity,
                OwnerBudget::global(),
            )?))
        }
    }
    struct CpalDevice(cpal::Device);
    struct CpalStream(cpal::Stream);
    impl StreamControl for CpalStream {
        fn play(&self) -> Result<(), String> {
            self.0.play().map_err(|error| error.to_string())
        }
        fn pause(&self) -> Result<(), String> {
            self.0.pause().map_err(|error| error.to_string())
        }
    }
    impl DeviceDriver for CpalDevice {
        type Stream = CpalStream;
        fn open(
            self,
            rate: u32,
            mut callback: DeviceCallback,
            errors: ErrorSignal,
        ) -> Result<CpalStream, String> {
            if callback.shared.lease.is_cancelled()
                || !callback.shared.alive.load(Ordering::Acquire)
            {
                return Err("Selected output preparation was cancelled".into());
            }
            let supported = self
                .0
                .supported_output_configs()
                .map_err(|error| error.to_string())?
                .any(|config| {
                    supports_exact_stereo_f32(
                        config.channels(),
                        config.sample_format() == cpal::SampleFormat::F32,
                        config.min_sample_rate().0,
                        config.max_sample_rate().0,
                        rate,
                    )
                });
            if !supported {
                return Err("Selected output does not support exact stereo F32 source rate".into());
            }
            if callback.shared.lease.is_cancelled()
                || !callback.shared.alive.load(Ordering::Acquire)
            {
                return Err("Selected output preparation was cancelled".into());
            }
            let config = cpal::StreamConfig {
                channels: 2,
                sample_rate: cpal::SampleRate(rate),
                buffer_size: cpal::BufferSize::Default,
            };
            let mut origin: Option<cpal::StreamInstant> = None;
            let stream = self
                .0
                .build_output_stream::<f32, _, _>(
                    &config,
                    move |samples, info| {
                        let time = info.timestamp();
                        let base = *origin.get_or_insert(time.callback);
                        let times = time
                            .callback
                            .duration_since(&base)
                            .zip(time.playback.duration_since(&base))
                            .and_then(|(callback, playback)| {
                                Some(CallbackTimes {
                                    callback_ns: callback.as_nanos().try_into().ok()?,
                                    playback_ns: playback.as_nanos().try_into().ok()?,
                                })
                            });
                        callback.render(samples, times);
                    },
                    move |_| errors.fail(),
                    None,
                )
                .map_err(|error| error.to_string())?;
            Ok(CpalStream(stream))
        }
    }
}
#[cfg(windows)]
pub use cpal_driver::CpalOutputFactory;
