//! Incremental stereo transport driven by injected, nonblocking worker adapters.
//! This supplies no decoder, device host, resampler, endpoint selection or driver.
//! It never connects music to the communication mixer. Construction is idle.
use super::{
    Track,
    playback::{BackendEvent, PlaybackBackend, PlaybackLease},
};

/// Bounds this transport's pending stereo PCM, not a device's internal buffers.
pub const MAX_BUFFER_FRAMES: usize = 1024;

#[derive(Clone, Debug, PartialEq)]
pub enum DecodedRead {
    Pending,
    Frames {
        sample_rate: u32,
        frames: Vec<[f32; 2]>,
    },
    End,
}

/// All methods run on the adapter worker, never on an audio callback. `begin`
/// accepts bounded preparation work; it must not synchronously block on file I/O.
/// Implementations check the lease before asynchronous source work/completions,
/// own any source access, and release partial resources themselves on an error.
pub trait DecoderFactory: Send {
    fn begin(&mut self, track: &Track, lease: PlaybackLease) -> Result<Box<dyn Decoder>, String>;
}

/// Incremental source work must be bounded and nonblocking. Return `Pending` while
/// worker I/O is incomplete. A Frames result contains 1..=max_frames exact stereo
/// frames at a known source rate; layout conversion is an adapter responsibility.
/// `release` cancels queued source work and drops source resources, independently
/// of whether the host has already revoked the lease. It must be nonblocking.
pub trait Decoder: Send {
    fn poll(&mut self, max_frames: usize) -> Result<DecodedRead, String>;
    fn release(&mut self);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputState {
    #[default]
    Pending,
    Running,
    Paused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputWrite {
    /// Backpressure: no frames were accepted and retrying later is permitted.
    Pending,
    /// Exactly this many leading frames were accepted; zero is a protocol error.
    Written(usize),
}

/// The caller configures an explicit endpoint outside this transport. `begin`
/// prepares without starting output, must be nonblocking, and must reject an
/// unsupported source rate/layout instead of resampling or choosing a fallback.
/// Partial-open failures must release their resources before returning an error.
pub trait OutputFactory: Send {
    fn begin(&mut self, sample_rate: u32, lease: PlaybackLease) -> Result<Box<dyn Output>, String>;
}

/// A worker-owned bounded output adapter. Methods must be nonblocking. A device
/// callback may consume an adapter's bounded PCM queue; it must never decode or
/// perform source/file work. Queued operations and writes must recheck their lease
/// and operation at consumption, not just when accepted by these methods.
///
/// `poll_state` acknowledges ONLY the supplied lease/operation. A previous start
/// or pause completion must return Pending for a new request, never be relabeled.
/// Pausing preserves already accepted PCM and its playback position. `try_write`
/// accepts only a prefix, must not accumulate an unbounded queue, and may not write
/// after revocation/output-gate closure. The adapter receives its lease at begin;
/// closed/superseded authority is Pending, not a terminal session error. A genuine
/// write/drain error is terminal for the session even across play/pause changes.
/// `drained` means ALL accepted PCM and real output have drained; an empty staging
/// queue or decoder EOF alone is insufficient. `clear` stops/flushes pending work;
/// `release` frees output resources. Both are nonblocking and safe after errors.
/// Already accepted hardware samples cannot be promised retractable here.
pub trait Output: Send {
    fn start(&mut self, lease: &PlaybackLease, operation: u64) -> Result<(), String>;
    fn pause(&mut self, lease: &PlaybackLease, operation: u64) -> Result<(), String>;
    fn poll_state(&mut self, lease: &PlaybackLease, operation: u64) -> Result<OutputState, String>;
    fn try_write(&mut self, frames: &[[f32; 2]]) -> Result<OutputWrite, String>;
    fn drained(&mut self) -> Result<bool, String>;
    fn clear(&mut self);
    fn release(&mut self);
}

#[derive(Clone, Copy)]
struct Request {
    operation: u64,
    state: OutputState,
    issued: bool,
    acknowledged: bool,
}

struct Session {
    lease: PlaybackLease,
    decoder: Box<dyn Decoder>,
    output: Option<Box<dyn Output>>,
    frames: Vec<[f32; 2]>,
    offset: usize,
    sample_rate: Option<u32>,
    ready: bool,
    eof: bool,
    request: Option<Request>,
}

impl Session {
    fn remaining(&self) -> usize {
        self.frames.len().saturating_sub(self.offset)
    }

    fn authorized(&self, request: Request) -> bool {
        self.lease.is_current(request.operation)
            && match request.state {
                OutputState::Running => self.lease.may_output(),
                OutputState::Paused => !self.lease.may_output(),
                OutputState::Pending => false,
            }
    }

    fn may_read(&self) -> bool {
        // The only gate-closed decoding is the first bounded priming block.
        (!self.ready && self.request.is_none() && self.lease.is_current(0))
            || self.request.is_some_and(|request| {
                request.state == OutputState::Running && self.authorized(request)
            })
    }

    fn read(&mut self) -> Result<DecodedRead, String> {
        let read = self.decoder.poll(MAX_BUFFER_FRAMES)?;
        if self.lease.is_cancelled() {
            return Ok(DecodedRead::Pending);
        }
        if let DecodedRead::Frames {
            sample_rate,
            frames,
        } = &read
        {
            if *sample_rate == 0 {
                return Err("decoder returned an unknown sample rate".into());
            }
            if frames.is_empty() || frames.len() > MAX_BUFFER_FRAMES {
                return Err("decoder returned an empty or oversized PCM block".into());
            }
            if frames.iter().flatten().any(|sample| !sample.is_finite()) {
                return Err("decoder returned nonfinite PCM".into());
            }
            if self.sample_rate.is_some_and(|rate| rate != *sample_rate) {
                return Err("source sample rate changed during playback".into());
            }
        }
        Ok(read)
    }

    fn retain_frames(&mut self, sample_rate: u32, frames: Vec<[f32; 2]>) {
        self.sample_rate = Some(sample_rate);
        self.frames = frames;
        self.offset = 0;
    }

    fn release(&mut self) {
        self.frames.clear();
        self.offset = 0;
        self.request = None;
        if let Some(mut output) = self.output.take() {
            output.clear();
            output.release();
        }
        self.decoder.release();
    }
}

/// One controller owns one backend and its monotonically generated leases.
/// Each poll performs at most one bounded decoder read and one bounded PCM write.
/// An adapter is injected explicitly; this type is not proof of native playback.
pub struct StreamingBackend<D: DecoderFactory, O: OutputFactory> {
    decoder_factory: D,
    output_factory: O,
    session: Option<Session>,
    failure: Option<(PlaybackLease, String)>,
}

impl<D: DecoderFactory, O: OutputFactory> StreamingBackend<D, O> {
    pub fn new(decoder_factory: D, output_factory: O) -> Self {
        Self {
            decoder_factory,
            output_factory,
            session: None,
            failure: None,
        }
    }

    /// Pending PCM in this transport only; excludes adapter/device-owned buffers.
    pub fn buffered_frames(&self) -> usize {
        self.session.as_ref().map_or(0, Session::remaining)
    }

    fn clear_session(&mut self) {
        if let Some(mut session) = self.session.take() {
            session.release();
        }
    }

    fn request(
        &mut self,
        lease: &PlaybackLease,
        operation: u64,
        state: OutputState,
    ) -> Result<(), String> {
        if !lease.is_current(operation) || (state == OutputState::Running) != lease.may_output() {
            return Ok(());
        }
        let Some(session) = self.session.as_mut() else {
            return Err("no prepared music stream".into());
        };
        if session.lease.generation() != lease.generation() || !session.lease.is_current(operation)
        {
            return Ok(());
        }
        if session
            .request
            .is_some_and(|request| request.operation == operation && request.state == state)
        {
            return Ok(());
        }
        session.request = Some(Request {
            operation,
            state,
            issued: false,
            acknowledged: false,
        });
        Ok(())
    }

    fn poll_session(&mut self, session: &mut Session) -> Result<Option<BackendEvent>, String> {
        let generation = session.lease.generation();
        if !session.ready {
            if !session.may_read() {
                return Ok(None);
            }
            return match session.read()? {
                DecodedRead::Pending => Ok(None),
                DecodedRead::End => Err("source contains no supported audio frames".into()),
                DecodedRead::Frames {
                    sample_rate,
                    frames,
                } => {
                    session.retain_frames(sample_rate, frames);
                    session.ready = true;
                    Ok(Some(BackendEvent::Ready { generation }))
                }
            };
        }
        let Some(mut request) = session.request else {
            return Ok(None);
        };
        if !session.authorized(request) {
            return Ok(None);
        }
        if !request.issued {
            if session.output.is_none() {
                if request.state != OutputState::Running {
                    return Err("cannot pause an unopened output".into());
                }
                let result = self.output_factory.begin(
                    session
                        .sample_rate
                        .ok_or("music stream has no source rate")?,
                    session.lease.clone(),
                );
                if !session.authorized(request) {
                    if let Ok(mut output) = result {
                        output.clear();
                        output.release();
                    }
                    return Ok(None);
                }
                session.output = Some(result?);
            }
            let output = session.output.as_mut().ok_or("music output is missing")?;
            let result = match request.state {
                OutputState::Running => output.start(&session.lease, request.operation),
                OutputState::Paused => output.pause(&session.lease, request.operation),
                OutputState::Pending => unreachable!(),
            };
            if !session.authorized(request) {
                return Ok(None);
            }
            result?;
            request.issued = true;
            session.request = Some(request);
        }
        let state = session
            .output
            .as_mut()
            .ok_or("music output is missing")?
            .poll_state(&session.lease, request.operation);
        if !session.authorized(request) {
            return Ok(None);
        }
        let state = state?;
        if state == OutputState::Pending {
            return Ok(None);
        }
        if state != request.state {
            if request.acknowledged {
                return Err("output changed state without the current request".into());
            }
            return Ok(None);
        }
        if !request.acknowledged {
            request.acknowledged = true;
            session.request = Some(request);
            return Ok(Some(match state {
                OutputState::Running => BackendEvent::Started {
                    generation,
                    operation: request.operation,
                },
                OutputState::Paused => BackendEvent::Paused {
                    generation,
                    operation: request.operation,
                },
                OutputState::Pending => unreachable!(),
            }));
        }
        if state == OutputState::Running {
            if session.remaining() == 0 && !session.eof {
                if !session.may_read() {
                    return Ok(None);
                }
                match session.read()? {
                    DecodedRead::Pending => return Ok(None),
                    DecodedRead::End => session.eof = true,
                    DecodedRead::Frames {
                        sample_rate,
                        frames,
                    } => {
                        session.retain_frames(sample_rate, frames);
                    }
                }
            }
            if !session.authorized(request) {
                return Ok(None);
            }
            if session.remaining() > 0 {
                let offered = session.remaining();
                let write = session
                    .output
                    .as_mut()
                    .ok_or("music output is missing")?
                    .try_write(&session.frames[session.offset..]);
                // The output adapter independently checks revocation when consuming
                // queued work. Do not acknowledge a superseded request here.
                if session.lease.is_cancelled() {
                    return Ok(None);
                }
                match write? {
                    OutputWrite::Pending => {}
                    OutputWrite::Written(count) if count > 0 && count <= offered => {
                        session.offset += count;
                        if session.remaining() == 0 {
                            session.frames.clear();
                            session.offset = 0;
                        }
                    }
                    OutputWrite::Written(_) => {
                        return Err("output reported an invalid accepted-frame count".into());
                    }
                }
            }
        }
        if session.eof && session.remaining() == 0 && session.authorized(request) {
            let drained = session
                .output
                .as_mut()
                .ok_or("music output is missing")?
                .drained()?;
            if drained && session.authorized(request) {
                return Ok(Some(BackendEvent::Ended { generation }));
            }
        }
        Ok(None)
    }
}

impl<D: DecoderFactory, O: OutputFactory> PlaybackBackend for StreamingBackend<D, O> {
    fn load(&mut self, track: &Track, lease: PlaybackLease) -> Result<(), String> {
        if lease.is_cancelled() || !lease.is_current(0) {
            return Ok(());
        }
        self.clear_session();
        self.failure = None;
        if !lease.is_current(0) {
            return Ok(());
        }
        let result = self.decoder_factory.begin(track, lease.clone());
        if !lease.is_current(0) {
            if let Ok(mut decoder) = result {
                decoder.release();
            }
            return Ok(());
        }
        match result {
            Ok(decoder) => {
                self.session = Some(Session {
                    lease,
                    decoder,
                    output: None,
                    frames: Vec::new(),
                    offset: 0,
                    sample_rate: None,
                    ready: false,
                    eof: false,
                    request: None,
                });
            }
            Err(error) => self.failure = Some((lease, error)),
        }
        Ok(())
    }

    fn play(&mut self, lease: &PlaybackLease, operation: u64) -> Result<(), String> {
        self.request(lease, operation, OutputState::Running)
    }

    fn pause(&mut self, lease: &PlaybackLease, operation: u64) -> Result<(), String> {
        self.request(lease, operation, OutputState::Paused)
    }

    fn release(&mut self, lease: &PlaybackLease) {
        if self
            .session
            .as_ref()
            .is_some_and(|session| session.lease.generation() == lease.generation())
        {
            self.clear_session();
        }
        if self
            .failure
            .as_ref()
            .is_some_and(|(pending, _)| pending.generation() == lease.generation())
        {
            self.failure = None;
        }
    }

    fn poll(&mut self) -> Option<BackendEvent> {
        if let Some((lease, error)) = self.failure.take()
            && !lease.is_cancelled()
        {
            return Some(BackendEvent::Failed {
                generation: lease.generation(),
                error: error.chars().take(512).collect(),
            });
        }
        let mut session = self.session.take()?;
        if session.lease.is_cancelled() {
            session.release();
            return None;
        }
        let result = self.poll_session(&mut session);
        if session.lease.is_cancelled() {
            session.release();
            return None;
        }
        match result {
            Ok(Some(event @ BackendEvent::Ended { .. })) => {
                session.release();
                Some(event)
            }
            Ok(event) => {
                self.session = Some(session);
                event
            }
            Err(error) => {
                let generation = session.lease.generation();
                session.release();
                Some(BackendEvent::Failed {
                    generation,
                    error: error.chars().take(512).collect(),
                })
            }
        }
    }
}

impl<D: DecoderFactory, O: OutputFactory> Drop for StreamingBackend<D, O> {
    fn drop(&mut self) {
        self.clear_session();
    }
}
