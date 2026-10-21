use crate::{AudioError, CommGains, CommPolicy, PRESET_TRANSITION_FRAMES, SAMPLE_RATE_HZ,
    TxState, VoicePreset, VoiceProcessor, VoiceState};

// 48,000 / 80 is exactly 600 frames. Keeping an integer phase avoids cumulative
// oscillator drift and gives identical samples for arbitrary block boundaries.
const ROBOT_PERIOD_FRAMES: u16 = 600;

impl Default for VoiceProcessor {
    fn default() -> Self {
        Self {
            phase_frame: 0,
            target: VoicePreset::Bypass,
            wet_weight: 0.0,
            transition_start: 0.0,
            transition_elapsed: PRESET_TRANSITION_FRAMES,
        }
    }
}

impl VoiceProcessor {
    pub fn new() -> Self { Self::default() }

    /// Normalized mono, fixed 48 kHz. Every error clears output and DSP history.
    /// Preset changes linearly crossfade over 240 frames, with target weight
    /// (transition_frame + 1) / 240. Both phase and fades continue across blocks.
    /// A change during a fade starts from the current wet weight, without a jump.
    pub fn process(&mut self, input: &[f32], preset: VoicePreset,
        sample_rate: u32, out: &mut [f32]) -> Result<(), AudioError> {
        out.fill(0.0);
        let error = if sample_rate != SAMPLE_RATE_HZ {
            Some(AudioError::InvalidSampleRate)
        } else if input.len() != out.len() {
            Some(AudioError::LengthMismatch)
        } else if !valid_samples(input) {
            Some(AudioError::InvalidSample)
        } else {
            None
        };
        if let Some(error) = error {
            *self = Self::default();
            return Err(error);
        }
        if preset != self.target {
            self.target = preset;
            self.transition_start = self.wet_weight;
            self.transition_elapsed = 0;
        }
        let target_weight = match preset { VoicePreset::Bypass => 0.0, VoicePreset::Robot => 1.0 };
        for (&dry, output) in input.iter().zip(out) {
            if self.transition_elapsed < PRESET_TRANSITION_FRAMES {
                self.transition_elapsed += 1;
                let alpha = self.transition_elapsed as f32 / PRESET_TRANSITION_FRAMES as f32;
                self.wet_weight = self.transition_start * (1.0 - alpha) + target_weight * alpha;
            }
            let phase = std::f64::consts::TAU * self.phase_frame as f64
                / ROBOT_PERIOD_FRAMES as f64;
            let wet = dry * phase.sin() as f32;
            *output = if self.wet_weight == 0.0 {
                // Preserve exact Bypass samples, including the sign of zero.
                dry
            } else if self.wet_weight == 1.0 {
                wet
            } else {
                (dry * (1.0 - self.wet_weight) + wet * self.wet_weight).clamp(-1.0, 1.0)
            };
            self.phase_frame = (self.phase_frame + 1) % ROBOT_PERIOD_FRAMES;
        }
        Ok(())
    }

    /// Clears oscillator and preset-transition history to the initial Bypass state.
    pub fn reset(&mut self) -> Result<(), AudioError> {
        *self = Self::default();
        Ok(())
    }
}

/// Half(L + R), with normalized input validation and fail-closed output.
pub fn downmix_stereo(interleaved: &[f32], sample_rate: u32,
    out: &mut [f32]) -> Result<(), AudioError> {
    out.fill(0.0);
    if sample_rate != SAMPLE_RATE_HZ { return Err(AudioError::InvalidSampleRate); }
    if out.len().checked_mul(2) != Some(interleaved.len()) {
        return Err(AudioError::LengthMismatch);
    }
    if !valid_samples(interleaved) { return Err(AudioError::InvalidSample); }
    for (stereo, output) in interleaved.chunks_exact(2).zip(out) {
        *output = ((stereo[0] + stereo[1]) * 0.5).clamp(-1.0, 1.0);
    }
    Ok(())
}

/// Mixes processed voice and owned clips. Music has no route into this bus.
/// Validation precedes policy gating, and every failure clears the entire output.
pub fn process_communication(processed_voice: &[f32], clips: &[f32],
    gains: CommGains, policy: CommPolicy, out: &mut [f32]) -> Result<(), AudioError> {
    out.fill(0.0);
    if processed_voice.len() != out.len() || clips.len() != out.len() {
        return Err(AudioError::LengthMismatch);
    }
    if !valid_gains(gains) { return Err(AudioError::InvalidGain); }
    if !valid_samples(processed_voice) || !valid_samples(clips) {
        return Err(AudioError::InvalidSample);
    }
    if policy.tx != TxState::Ready { return Ok(()); }
    let voice_open = policy.voice == VoiceState::Enabled && policy.ptt_open;
    for ((&voice, &clip), output) in processed_voice.iter().zip(clips).zip(out) {
        let voice = if voice_open { voice * gains.voice } else { 0.0 };
        *output = (voice + clip * gains.clips).clamp(-1.0, 1.0);
    }
    Ok(())
}

pub(crate) fn valid_samples(samples: &[f32]) -> bool {
    samples.iter().all(|sample| sample.is_finite() && (-1.0..=1.0).contains(sample))
}

pub(crate) fn valid_gains(gains: CommGains) -> bool {
    [gains.voice, gains.clips].iter().all(|gain| gain.is_finite() && (0.0..=1.0).contains(gain))
}
