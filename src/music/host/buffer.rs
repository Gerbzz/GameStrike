//! Safe, bounded stereo SPSC storage. No allocation or locks while copying PCM.
use std::sync::{
    Arc,
    atomic::{AtomicU32, AtomicU64, Ordering},
};

pub const MAX_RING_FRAMES: usize = 4096;

struct Ring {
    frames: Box<[[AtomicU32; 2]]>,
    written: AtomicU64,
    read: AtomicU64,
}

/// One producer and one consumer are returned; neither endpoint is Clone.
pub struct Producer(Arc<Ring>);
pub struct Consumer(Arc<Ring>);
pub struct PreparedRead {
    ring: Arc<Ring>,
    start: u64,
    count: usize,
}
impl PreparedRead {
    pub fn frames(&self) -> usize { self.count }
}

pub fn stereo_ring(capacity: usize) -> Result<(Producer, Consumer), String> {
    if capacity == 0 || capacity > MAX_RING_FRAMES || !capacity.is_power_of_two() {
        return Err("PCM ring capacity must be a power of two between 1 and 4096 frames".into());
    }
    let frames = (0..capacity)
        .map(|_| [AtomicU32::new(0), AtomicU32::new(0)])
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let ring = Arc::new(Ring { frames, written: AtomicU64::new(0), read: AtomicU64::new(0) });
    Ok((Producer(ring.clone()), Consumer(ring)))
}

impl Producer {
    pub fn queued(&self) -> usize {
        let written = self.0.written.load(Ordering::Acquire);
        let read = self.0.read.load(Ordering::Acquire);
        written.saturating_sub(read).min(self.0.frames.len() as u64) as usize
    }
    pub fn written(&self) -> u64 { self.0.written.load(Ordering::Acquire) }
    pub fn consumed(&self) -> u64 { self.0.read.load(Ordering::Acquire) }

    pub fn write(&mut self, frames: &[[f32; 2]]) -> Result<usize, String> {
        if frames.len() > MAX_RING_FRAMES {
            return Err("Output PCM offer exceeds the bounded frame limit".into());
        }
        if frames.iter().flatten().any(|sample| !sample.is_finite()) {
            return Err("Output PCM contains a nonfinite sample".into());
        }
        let written = self.0.written.load(Ordering::Relaxed);
        let count = frames.len().min(self.0.frames.len().saturating_sub(self.queued()));
        let end = written.checked_add(count as u64).ok_or("PCM frame counter is exhausted")?;
        for (offset, frame) in frames.iter().take(count).enumerate() {
            let index = ((written + offset as u64) % self.0.frames.len() as u64) as usize;
            self.0.frames[index][0].store(frame[0].to_bits(), Ordering::Relaxed);
            self.0.frames[index][1].store(frame[1].to_bits(), Ordering::Relaxed);
        }
        self.0.written.store(end, Ordering::Release);
        Ok(count)
    }
}

impl Consumer {
    /// Copy without freeing slots. The callback rechecks authority before commit;
    /// abandoned preparation leaves the queued playback position intact.
    pub fn prepare(&mut self, interleaved: &mut [f32]) -> PreparedRead {
        let start = self.0.read.load(Ordering::Relaxed);
        let written = self.0.written.load(Ordering::Acquire);
        let count = (written - start).min((interleaved.len() / 2) as u64) as usize;
        for (offset, target) in interleaved.chunks_exact_mut(2).take(count).enumerate() {
            let index = ((start + offset as u64) % self.0.frames.len() as u64) as usize;
            target[0] = f32::from_bits(self.0.frames[index][0].load(Ordering::Relaxed));
            target[1] = f32::from_bits(self.0.frames[index][1].load(Ordering::Relaxed));
        }
        PreparedRead { ring: self.0.clone(), start, count }
    }

    pub fn commit(&mut self, ticket: PreparedRead) -> bool {
        if !Arc::ptr_eq(&self.0, &ticket.ring)
            || self.0.read.load(Ordering::Relaxed) != ticket.start
        { return false; }
        let Some(end) = ticket.start.checked_add(ticket.count as u64) else { return false };
        if end > self.0.written.load(Ordering::Acquire) { return false; }
        self.0.read.store(end, Ordering::Release);
        true
    }

    pub fn written(&self) -> u64 { self.0.written.load(Ordering::Acquire) }
    pub fn consumed(&self) -> u64 { self.0.read.load(Ordering::Acquire) }
}
