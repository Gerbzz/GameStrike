use crate::lifecycle::append_bounded;
use crate::signal::valid_samples;
use crate::{
    AudioError, BridgeBlock, BridgeStatus, DeviceSamples, EndpointAvailability, EndpointBackend,
    EndpointBridge, EndpointPairId, NegotiatedFormat, SAMPLE_RATE_HZ, SampleEncoding,
    VerifiedEndpoint,
};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

impl VerifiedEndpoint {
    pub fn pair(&self) -> &EndpointPairId {
        &self.pair
    }
    pub fn is_available(&self) -> bool {
        self.live.load(Ordering::Acquire)
    }
}

impl<B: EndpointBackend> EndpointBridge<B> {
    pub fn new(backend: B, capacity_frames: usize) -> Self {
        Self {
            backend,
            capacity_frames,
            queue: VecDeque::new(),
            format: None,
            proof: None,
            backend_open: false,
        }
    }

    pub fn availability(&self) -> EndpointAvailability {
        if self
            .proof
            .as_ref()
            .is_some_and(VerifiedEndpoint::is_available)
        {
            EndpointAvailability::Verified
        } else {
            EndpointAvailability::Unavailable
        }
    }

    pub fn queued_frames(&self) -> usize {
        self.queue.len()
    }

    /// Replacing a connection permanently revokes its old capability first.
    /// Only backend-observed stable IDs and negotiated format authorize a proof.
    pub fn connect(&mut self, pair: &EndpointPairId) -> Result<VerifiedEndpoint, AudioError> {
        self.close_connection();
        if pair.render.trim().is_empty() || pair.capture.trim().is_empty() {
            return Err(AudioError::EndpointProofRejected);
        }
        let handshake = match self.backend.handshake(pair) {
            Ok(handshake) => handshake,
            Err(error) => {
                self.backend.close();
                return Err(error);
            }
        };
        self.backend_open = true;
        if handshake.opened_pair != *pair || handshake.backend_session_id.trim().is_empty() {
            self.close_connection();
            return Err(AudioError::EndpointProofRejected);
        }
        if !supported_format(handshake.format) {
            self.close_connection();
            return Err(AudioError::UnsupportedFormat);
        }
        let proof = VerifiedEndpoint {
            pair: pair.clone(),
            live: Arc::new(AtomicBool::new(true)),
        };
        self.format = Some(handshake.format);
        self.proof = Some(proof.clone());
        Ok(proof)
    }

    pub fn enqueue(&mut self, mono: &[f32]) -> Result<(), AudioError> {
        if !valid_samples(mono) {
            self.queue.clear();
            return Err(AudioError::InvalidSample);
        }
        if self.availability() != EndpointAvailability::Verified {
            self.queue.clear();
            return Err(AudioError::EndpointUnavailable);
        }
        append_bounded(&mut self.queue, mono, self.capacity_frames)
    }

    /// Pumps at most capacity_frames in the last actually negotiated format.
    /// A partial block underflows to all-silence and discards incomplete samples.
    /// Any backend write error revokes proof, flushes data, and returns silence.
    /// Physical silence on an unavailable device remains a host responsibility.
    pub fn pump(&mut self, frames: usize) -> Result<BridgeBlock, AudioError> {
        let format = self.format.ok_or(AudioError::EndpointUnavailable)?;
        if frames > self.capacity_frames {
            self.queue.clear();
            return Err(AudioError::QueueFull);
        }
        let mut samples = match silent_block(format, frames) {
            Ok(samples) => samples,
            Err(error) => {
                self.close_connection();
                return Err(error);
            }
        };
        if self.availability() != EndpointAvailability::Verified {
            self.queue.clear();
            return Ok(BridgeBlock {
                samples,
                status: BridgeStatus::Disconnected,
            });
        }
        let status = if self.queue.len() < frames {
            self.queue.clear();
            BridgeStatus::SilentUnderflow
        } else {
            let channels = format.channels as usize;
            for frame in 0..frames {
                let mono = self.queue.pop_front().unwrap_or(0.0);
                write_frame(&mut samples, frame, channels, mono);
            }
            BridgeStatus::Ready
        };
        if self.backend.write(format, &samples).is_err() {
            clear_samples(&mut samples);
            self.close_connection();
            return Ok(BridgeBlock {
                samples,
                status: BridgeStatus::Disconnected,
            });
        }
        Ok(BridgeBlock { samples, status })
    }

    pub fn disconnect(&mut self) -> Result<(), AudioError> {
        self.close_connection();
        Ok(())
    }

    fn close_connection(&mut self) {
        if let Some(proof) = self.proof.take() {
            proof.live.store(false, Ordering::Release);
        }
        self.queue.clear();
        if self.backend_open {
            self.backend_open = false;
            self.backend.close();
        }
        // Keep only the negotiated format so disconnected calls can return typed
        // silence. A new handshake, never the old token, is required to write.
    }
}

impl<B: EndpointBackend> Drop for EndpointBridge<B> {
    fn drop(&mut self) {
        self.close_connection();
    }
}

fn supported_format(format: NegotiatedFormat) -> bool {
    format.sample_rate == SAMPLE_RATE_HZ
        && matches!(format.channels, 1 | 2)
        && matches!(
            format.encoding,
            SampleEncoding::F32 | SampleEncoding::I16 | SampleEncoding::I32
        )
}

fn silent_block(format: NegotiatedFormat, frames: usize) -> Result<DeviceSamples, AudioError> {
    let count = frames
        .checked_mul(format.channels as usize)
        .ok_or(AudioError::LengthMismatch)?;
    match format.encoding {
        SampleEncoding::F32 => Ok(DeviceSamples::F32(zeroed_vec(count, 0.0)?)),
        SampleEncoding::I16 => Ok(DeviceSamples::I16(zeroed_vec(count, 0)?)),
        SampleEncoding::I32 => Ok(DeviceSamples::I32(zeroed_vec(count, 0)?)),
        SampleEncoding::Unsupported => Err(AudioError::UnsupportedFormat),
    }
}

fn zeroed_vec<T: Clone>(count: usize, zero: T) -> Result<Vec<T>, AudioError> {
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(count)
        .map_err(|_| AudioError::QueueFull)?;
    samples.resize(count, zero);
    Ok(samples)
}

fn write_frame(samples: &mut DeviceSamples, frame: usize, channels: usize, mono: f32) {
    let start = frame * channels;
    let end = start + channels;
    match samples {
        DeviceSamples::F32(samples) => samples[start..end].fill(mono),
        DeviceSamples::I16(samples) => {
            let value = (mono as f64 * 32_768.0)
                .round()
                .clamp(i16::MIN as f64, i16::MAX as f64) as i16;
            samples[start..end].fill(value);
        }
        DeviceSamples::I32(samples) => {
            let value = (mono as f64 * 2_147_483_648.0)
                .round()
                .clamp(i32::MIN as f64, i32::MAX as f64) as i32;
            samples[start..end].fill(value);
        }
    }
}

fn clear_samples(samples: &mut DeviceSamples) {
    match samples {
        DeviceSamples::F32(samples) => samples.fill(0.0),
        DeviceSamples::I16(samples) => samples.fill(0),
        DeviceSamples::I32(samples) => samples.fill(0),
    }
}
