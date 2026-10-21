#![forbid(unsafe_code)]
//! Safe 48 kHz communication DSP with synthetic, injected host boundaries.
//! This crate performs no device discovery, capture, driver installation, or OS changes.
//! Endpoint proof establishes only the injected backend's synthetic connection.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

mod endpoint;
mod lifecycle;
mod signal;

pub use signal::{downmix_stereo, process_communication};

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

#[derive(Debug)]
pub struct VoiceProcessor {
    phase_frame: u16,
    target: VoicePreset,
    wet_weight: f32,
    transition_start: f32,
    transition_elapsed: usize,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputDeviceId(pub String);

/// Implementations belong to the host. The core provides no microphone backend.
/// `release` must safely clean up an unsuccessful or partially completed open.
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

/// Synthetic host lifecycle. Render consumes live voice even when PTT is closed,
/// advances its DSP history, and discards its contribution without releasing input.
/// Each render rechecks endpoint proof. Revocation faults on that render, clears
/// output, releases input, flushes both queues, and clears effects to Bypass.
/// Invalid queued samples or render gains also release/flush/reset and latch fault.
/// A faulted session cannot be revived by endpoint, voice, preset, or TX changes.
pub struct Session<I: MicInput> {
    input: I,
    input_device: Option<InputDeviceId>,
    endpoint: Option<VerifiedEndpoint>,
    voice: VoiceState,
    tx: TxState,
    preset: VoicePreset,
    processor: VoiceProcessor,
    voice_queue: VecDeque<f32>,
    clip_queue: VecDeque<f32>,
    capacity_frames: usize,
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
/// Handshake must open the requested stable pair and report the actual negotiated
/// format. `close` must safely clean up unsuccessful or partially completed opens.
/// Writes consume a whole block or return an error; physical output after a backend
/// failure cannot be established by this synthetic contract.
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
    live: Arc<AtomicBool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BridgeStatus { Ready, SilentUnderflow, Disconnected }

#[derive(Clone, Debug, PartialEq)]
pub struct BridgeBlock {
    pub samples: DeviceSamples,
    pub status: BridgeStatus,
}

pub struct EndpointBridge<B: EndpointBackend> {
    backend: B,
    capacity_frames: usize,
    queue: VecDeque<f32>,
    format: Option<NegotiatedFormat>,
    proof: Option<VerifiedEndpoint>,
    backend_open: bool,
}
