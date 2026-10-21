#![forbid(unsafe_code)]
//! Tests-first contract scaffold. Processing and lifecycle operations deliberately
//! return `AudioError::Unimplemented` until hosted behavioral RED is reviewed.
//! This crate performs no device discovery, capture, driver installation, or OS changes.

use std::fmt;

pub const SAMPLE_RATE_HZ: u32 = 48_000;
pub const PRESET_TRANSITION_FRAMES: usize = 240;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioError {
    Unimplemented,
    InvalidSampleRate,
    LengthMismatch,
    InvalidSample,
    InvalidGain,
    UnsupportedFormat,
    EndpointUnavailable,
    EndpointProofRejected,
    BackendDisconnected,
    QueueFull,
    Faulted,
    InputFailure,
    VoiceDisabled,
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for AudioError {}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VoicePreset {
    #[default]
    Bypass,
    Robot,
}

#[derive(Debug, Default)]
pub struct VoiceProcessor;

impl VoiceProcessor {
    pub fn new() -> Self { Self }

    /// Normalized mono, fixed 48 kHz. An error must clear the entire output.
    pub fn process(&mut self, _input: &[f32], _preset: VoicePreset,
        _sample_rate: u32, _out: &mut [f32]) -> Result<(), AudioError> {
        Err(AudioError::Unimplemented)
    }

    /// Clears oscillator and preset-transition history.
    pub fn reset(&mut self) -> Result<(), AudioError> {
        Err(AudioError::Unimplemented)
    }
}

/// Half(L + R), with normalized input validation and fail-closed output.
pub fn downmix_stereo(_interleaved: &[f32], _sample_rate: u32,
    _out: &mut [f32]) -> Result<(), AudioError> {
    Err(AudioError::Unimplemented)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VoiceState {
    #[default]
    Disabled,
    Enabled,
    Muted,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TxState {
    #[default]
    Ready,
    Muted,
    Faulted,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CommGains {
    pub voice: f32,
    pub clips: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommPolicy {
    pub voice: VoiceState,
    pub ptt_open: bool,
    pub tx: TxState,
}

/// Only processed voice and owned clips belong on the communication bus.
/// Both inputs must match output length; every failure clears output.
pub fn process_communication(_processed_voice: &[f32], _clips: &[f32],
    _gains: CommGains, _policy: CommPolicy, _out: &mut [f32]) -> Result<(), AudioError> {
    Err(AudioError::Unimplemented)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputDeviceId(pub String);

/// Implementations belong to the host. Tests use a trace-only fake.
pub trait MicInput {
    fn open(&mut self, device: &InputDeviceId) -> Result<(), AudioError>;
    fn release(&mut self);
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EndpointAvailability {
    #[default]
    Unavailable,
    Verified,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SessionSnapshot {
    pub voice: VoiceState,
    pub tx: TxState,
    pub endpoint: EndpointAvailability,
    pub queued_voice_frames: usize,
    pub queued_clip_frames: usize,
    pub preset: VoicePreset,
}

pub struct Session<I: MicInput> {
    _input: I,
}

impl<I: MicInput> Session<I> {
    pub fn new(input: I, _capacity_frames: usize) -> Self { Self { _input: input } }
    pub fn snapshot(&self) -> SessionSnapshot { SessionSnapshot::default() }
    pub fn set_endpoint(&mut self, _endpoint: Option<VerifiedEndpoint>) -> Result<(), AudioError> {
        Err(AudioError::Unimplemented)
    }
    pub fn enable_voice(&mut self, _device: &InputDeviceId) -> Result<(), AudioError> {
        Err(AudioError::Unimplemented)
    }
    pub fn disable_voice(&mut self) -> Result<(), AudioError> { Err(AudioError::Unimplemented) }
    pub fn mute_voice(&mut self) -> Result<(), AudioError> { Err(AudioError::Unimplemented) }
    pub fn mute_tx(&mut self) -> Result<(), AudioError> { Err(AudioError::Unimplemented) }
    pub fn resume_tx(&mut self) -> Result<(), AudioError> { Err(AudioError::Unimplemented) }
    pub fn fault(&mut self) -> Result<(), AudioError> { Err(AudioError::Unimplemented) }
    pub fn set_preset(&mut self, _preset: VoicePreset) -> Result<(), AudioError> {
        Err(AudioError::Unimplemented)
    }
    pub fn enqueue_voice(&mut self, _mono: &[f32]) -> Result<(), AudioError> {
        Err(AudioError::Unimplemented)
    }
    pub fn enqueue_clip(&mut self, _mono: &[f32]) -> Result<(), AudioError> {
        Err(AudioError::Unimplemented)
    }
    pub fn render(&mut self, _gains: CommGains, _ptt_open: bool,
        _out: &mut [f32]) -> Result<(), AudioError> {
        Err(AudioError::Unimplemented)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EndpointPairId {
    pub render: String,
    pub capture: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SampleEncoding { F32, I16, I32, Unsupported }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NegotiatedFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub encoding: SampleEncoding,
}

/// Synthetic evidence returned by the backend after opening the exact stable pair.
/// A matching display name alone is never evidence. No driver trust claim is made.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendHandshake {
    pub opened_pair: EndpointPairId,
    pub backend_session_id: String,
    pub format: NegotiatedFormat,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DeviceSamples { F32(Vec<f32>), I16(Vec<i16>), I32(Vec<i32>) }

/// The host implements this boundary; the core has no actual device backend.
pub trait EndpointBackend {
    fn handshake(&mut self, requested: &EndpointPairId) -> Result<BackendHandshake, AudioError>;
    fn write(&mut self, format: NegotiatedFormat, samples: &DeviceSamples) -> Result<(), AudioError>;
    fn close(&mut self);
}

/// Opaque capability minted only after backend proof and format verification.
/// Implemented handles must become unavailable after the issuing bridge disconnects.
#[derive(Clone, Debug)]
pub struct VerifiedEndpoint {
    pair: EndpointPairId,
}

impl VerifiedEndpoint {
    pub fn pair(&self) -> &EndpointPairId { &self.pair }
    pub fn is_available(&self) -> bool { false }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BridgeStatus { Ready, SilentUnderflow, Disconnected }

#[derive(Clone, Debug, PartialEq)]
pub struct BridgeBlock {
    pub samples: DeviceSamples,
    pub status: BridgeStatus,
}

pub struct EndpointBridge<B: EndpointBackend> { _backend: B }

impl<B: EndpointBackend> EndpointBridge<B> {
    pub fn new(backend: B, _capacity_frames: usize) -> Self { Self { _backend: backend } }
    pub fn availability(&self) -> EndpointAvailability { EndpointAvailability::Unavailable }
    pub fn queued_frames(&self) -> usize { 0 }
    pub fn connect(&mut self, _pair: &EndpointPairId) -> Result<VerifiedEndpoint, AudioError> {
        Err(AudioError::Unimplemented)
    }
    pub fn enqueue(&mut self, _mono: &[f32]) -> Result<(), AudioError> {
        Err(AudioError::Unimplemented)
    }
    /// Writes the negotiated format. Underflow is silence; disconnect flushes queued data.
    pub fn pump(&mut self, _frames: usize) -> Result<BridgeBlock, AudioError> {
        Err(AudioError::Unimplemented)
    }
    pub fn disconnect(&mut self) -> Result<(), AudioError> { Err(AudioError::Unimplemented) }
}
