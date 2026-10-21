use crate::{AudioError, CommGains, CommPolicy, EndpointAvailability, InputDeviceId, MicInput,
    SAMPLE_RATE_HZ, Session, SessionSnapshot, TxState, VerifiedEndpoint, VoicePreset,
    VoiceProcessor, VoiceState, process_communication};
use crate::signal::{valid_gains, valid_samples};
use std::collections::VecDeque;

impl<I: MicInput> Session<I> {
    /// Creates an idle session without opening input or discovering any endpoint.
    /// Each synthetic input queue has the caller-specified maximum frame count.
    pub fn new(input: I, capacity_frames: usize) -> Self {
        Self {
            input,
            input_device: None,
            endpoint: None,
            voice: VoiceState::Disabled,
            tx: TxState::Ready,
            preset: VoicePreset::Bypass,
            processor: VoiceProcessor::new(),
            voice_queue: VecDeque::new(),
            clip_queue: VecDeque::new(),
            capacity_frames,
        }
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            voice: self.voice,
            tx: self.tx,
            endpoint: if self.endpoint.as_ref().is_some_and(VerifiedEndpoint::is_available) {
                EndpointAvailability::Verified
            } else {
                EndpointAvailability::Unavailable
            },
            queued_voice_frames: self.voice_queue.len(),
            queued_clip_frames: self.clip_queue.len(),
            preset: self.preset,
        }
    }

    /// Installs only a live, backend-verified capability. It never opens input.
    /// Removing a previously installed endpoint is a terminal endpoint-loss fault.
    pub fn set_endpoint(&mut self, endpoint: Option<VerifiedEndpoint>) -> Result<(), AudioError> {
        self.ensure_not_faulted()?;
        let Some(endpoint) = endpoint else {
            if self.endpoint.take().is_some() { self.enter_fault(); }
            return Ok(());
        };
        if self.endpoint.as_ref().is_some_and(|proof| !proof.is_available()) {
            self.endpoint = None;
            self.enter_fault();
            return Err(AudioError::Faulted);
        }
        if !endpoint.is_available() {
            if self.endpoint.take().is_some() { self.enter_fault(); }
            return Err(AudioError::EndpointUnavailable);
        }
        // Replacing a connection requires a new explicit voice-enable action.
        if self.endpoint.is_some() {
            self.release_input();
            self.clear_audio();
            self.voice = VoiceState::Disabled;
        }
        self.endpoint = Some(endpoint);
        Ok(())
    }

    /// Opens exactly the selected input after verifying endpoint and TX readiness.
    /// No default-device lookup, substitute, or automatic reopen exists here.
    pub fn enable_voice(&mut self, device: &InputDeviceId) -> Result<(), AudioError> {
        self.ensure_not_faulted()?;
        self.ensure_endpoint()?;
        if self.tx != TxState::Ready { return Err(AudioError::VoiceDisabled); }
        if device.0.trim().is_empty() {
            self.enter_fault();
            return Err(AudioError::InputFailure);
        }
        if self.input_device.as_ref() == Some(device) && self.voice == VoiceState::Enabled {
            return Ok(());
        }
        if self.input_device.is_some() {
            self.release_input();
            self.clear_audio();
        }
        if let Err(error) = self.input.open(device) {
            // A host can have partially opened input before reporting failure.
            self.input.release();
            self.enter_fault();
            return Err(error);
        }
        self.input_device = Some(device.clone());
        self.voice = VoiceState::Enabled;
        // The host open may have allowed a concurrent endpoint loss to surface.
        // Rechecking also releases a just-opened input if its proof was revoked.
        self.ensure_endpoint()?;
        Ok(())
    }

    pub fn disable_voice(&mut self) -> Result<(), AudioError> {
        self.ensure_not_faulted()?;
        self.release_input();
        self.clear_audio();
        self.voice = VoiceState::Disabled;
        Ok(())
    }

    pub fn mute_voice(&mut self) -> Result<(), AudioError> {
        self.ensure_not_faulted()?;
        self.release_input();
        self.clear_audio();
        self.voice = VoiceState::Muted;
        Ok(())
    }

    pub fn mute_tx(&mut self) -> Result<(), AudioError> {
        self.ensure_not_faulted()?;
        self.release_input();
        self.clear_audio();
        self.voice = VoiceState::Disabled;
        self.tx = TxState::Muted;
        Ok(())
    }

    pub fn resume_tx(&mut self) -> Result<(), AudioError> {
        self.ensure_not_faulted()?;
        self.ensure_endpoint()?;
        self.tx = TxState::Ready;
        // Voice stays disabled; a later explicit enable must select input again.
        Ok(())
    }

    pub fn fault(&mut self) -> Result<(), AudioError> {
        self.enter_fault();
        Ok(())
    }

    pub fn set_preset(&mut self, preset: VoicePreset) -> Result<(), AudioError> {
        self.ensure_not_faulted()?;
        self.check_revocation()?;
        self.preset = preset;
        Ok(())
    }

    pub fn enqueue_voice(&mut self, mono: &[f32]) -> Result<(), AudioError> {
        self.ensure_not_faulted()?;
        self.validate_queued_samples(mono)?;
        self.ensure_endpoint()?;
        if self.voice != VoiceState::Enabled || self.input_device.is_none()
            || self.tx != TxState::Ready {
            return Err(AudioError::VoiceDisabled);
        }
        append_bounded(&mut self.voice_queue, mono, self.capacity_frames)
    }

    pub fn enqueue_clip(&mut self, mono: &[f32]) -> Result<(), AudioError> {
        self.ensure_not_faulted()?;
        self.validate_queued_samples(mono)?;
        self.ensure_endpoint()?;
        // Muted TX discards new clips instead of accumulating later playback.
        if self.tx != TxState::Ready { return Ok(()); }
        append_bounded(&mut self.clip_queue, mono, self.capacity_frames)
    }

    /// Fills the caller's mono buffer; no block-sized allocation is performed.
    /// A revoked proof is reported once as EndpointUnavailable and latches fault.
    /// Subsequent faulted renders return silence without reopening any resource.
    pub fn render(&mut self, gains: CommGains, ptt_open: bool,
        out: &mut [f32]) -> Result<(), AudioError> {
        out.fill(0.0);
        if self.tx == TxState::Faulted { return Ok(()); }
        if !valid_gains(gains) {
            self.enter_fault();
            return Err(AudioError::InvalidGain);
        }
        self.ensure_endpoint()?;
        if self.tx == TxState::Muted { return Ok(()); }
        if let Err(error) = self.render_ready(gains, ptt_open, out) {
            out.fill(0.0);
            self.enter_fault();
            return Err(error);
        }
        if !ptt_open {
            // Any queued backlog was captured under the closed gate. It cannot
            // become audible later; DSP phase follows rendered frames only.
            self.voice_queue.clear();
        }
        Ok(())
    }

    fn render_ready(&mut self, gains: CommGains, ptt_open: bool,
        out: &mut [f32]) -> Result<(), AudioError> {
        let policy = CommPolicy { voice: self.voice, ptt_open, tx: self.tx };
        for output in out {
            let voice = self.voice_queue.pop_front().unwrap_or(0.0);
            let clip = self.clip_queue.pop_front().unwrap_or(0.0);
            let mut processed = [0.0];
            if self.voice == VoiceState::Enabled {
                self.processor.process(&[voice], self.preset, SAMPLE_RATE_HZ, &mut processed)?;
            }
            process_communication(&processed, &[clip], gains, policy, std::slice::from_mut(output))?;
        }
        Ok(())
    }

    fn ensure_not_faulted(&self) -> Result<(), AudioError> {
        if self.tx == TxState::Faulted { Err(AudioError::Faulted) } else { Ok(()) }
    }

    fn check_revocation(&mut self) -> Result<(), AudioError> {
        if self.endpoint.as_ref().is_some_and(|proof| !proof.is_available()) {
            self.endpoint = None;
            self.enter_fault();
            Err(AudioError::EndpointUnavailable)
        } else {
            Ok(())
        }
    }

    fn ensure_endpoint(&mut self) -> Result<(), AudioError> {
        self.check_revocation()?;
        if self.endpoint.is_some() { Ok(()) } else { Err(AudioError::EndpointUnavailable) }
    }

    fn validate_queued_samples(&mut self, mono: &[f32]) -> Result<(), AudioError> {
        if valid_samples(mono) { return Ok(()); }
        self.enter_fault();
        Err(AudioError::InvalidSample)
    }

    fn release_input(&mut self) {
        if self.input_device.take().is_some() { self.input.release(); }
    }

    fn clear_audio(&mut self) {
        self.voice_queue.clear();
        self.clip_queue.clear();
        self.processor = VoiceProcessor::new();
        self.preset = VoicePreset::Bypass;
    }

    fn enter_fault(&mut self) {
        self.release_input();
        self.clear_audio();
        self.voice = VoiceState::Disabled;
        self.tx = TxState::Faulted;
    }
}

impl<I: MicInput> Drop for Session<I> {
    fn drop(&mut self) { self.release_input(); }
}

/// Capacity rejection preserves the previously accepted queue atomically.
pub(crate) fn append_bounded(queue: &mut VecDeque<f32>, mono: &[f32],
    capacity: usize) -> Result<(), AudioError> {
    if mono.len() > capacity.saturating_sub(queue.len()) { return Err(AudioError::QueueFull); }
    queue.try_reserve(mono.len()).map_err(|_| AudioError::QueueFull)?;
    queue.extend(mono.iter().copied());
    Ok(())
}
