//! Incremental local-file decoding with Symphonia 0.5.5.
//! Contiguous raw MP3 completion uses exact successfully decoded packet-byte
//! coverage, not estimated duration. Skipped tags/info/junk remain fail-closed.
//! The proof depends on the pinned demuxer's forward, disjoint packets.
//! Lexical path checks do not attest mounted/reparse storage provenance.

use crate::music::{
    Track,
    playback::PlaybackLease,
    streaming::{DecodedRead, Decoder, DecoderFactory, MAX_BUFFER_FRAMES},
};
use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom},
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread,
    time::Duration,
};
use symphonia::core::{
    audio::{Channels, SampleBuffer},
    codecs::{self, CodecParameters, Decoder as CodecDecoder, DecoderOptions},
    errors::Error as SymphoniaError,
    formats::{FormatOptions, FormatReader},
    io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions, ReadBytes},
    meta::{Limit, MetadataOptions},
    probe::Hint,
};

pub const MAX_PACKET_BYTES: usize = 1024 * 1024;
pub const MAX_DECODED_FRAMES: usize = 65_536;
/// Released handles do not free a slot until their worker actually exits. This
/// process-wide bound includes workers stuck in OS/reader I/O across track switches.
pub const MAX_DECODER_WORKERS: usize = 4;
const MAX_HEADER_CHUNKS: usize = 256;
const SOURCE_BUFFER_BYTES: usize = 64 * 1024;
static ACTIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);

struct WorkerSlot;

impl WorkerSlot {
    fn acquire() -> Result<Self, String> {
        ACTIVE_WORKERS
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |active| {
                (active < MAX_DECODER_WORKERS).then_some(active + 1)
            })
            .map(|_| Self)
            .map_err(|_| "decoder worker limit reached; an earlier reader may still be blocked".into())
    }
}

impl Drop for WorkerSlot {
    fn drop(&mut self) { ACTIVE_WORKERS.fetch_sub(1, Ordering::SeqCst); }
}

struct Control {
    alive: AtomicBool,
    primed: AtomicBool,
    lease: PlaybackLease,
    wake: Condvar,
    wait: Mutex<()>,
    read_remaining: AtomicUsize,
}

impl Control {
    fn live(&self) -> bool {
        self.alive.load(Ordering::SeqCst) && !self.lease.is_cancelled()
    }

    fn authorized(&self, primed: bool) -> bool {
        self.live() && (self.lease.may_output() || (!primed && self.lease.is_current(0)))
    }

    // Only a decoder worker waits here. No catalog, UI or source-authority lock
    // is held across I/O. Release wakes this wait; it cannot interrupt kernel I/O.
    fn permission(&self) -> io::Result<()> {
        loop {
            if !self.live() {
                // read_exact/Symphonia retry Interrupted. Cancellation must be a
                // nonretryable error so a released reader cannot spin forever.
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "decoder was released"));
            }
            if self.authorized(self.primed.load(Ordering::SeqCst)) { return Ok(()); }
            let guard = self.wait.lock().map_err(|_| io::Error::other("decoder wait poisoned"))?;
            let _waited = self.wake.wait_timeout(guard, Duration::from_millis(100))
                .map_err(|_| io::Error::other("decoder wait poisoned"))?;
        }
    }

    fn close(&self) {
        self.alive.store(false, Ordering::SeqCst);
        self.wake.notify_all();
    }
}

/// Constructor opens nothing. File paths are consumed only by `begin` on explicit
/// controller load; even metadata/open/probing occur on the bounded decoder worker.
#[derive(Default)]
pub struct SymphoniaDecoderFactory;

impl SymphoniaDecoderFactory {
    pub fn new() -> Self { Self }

    pub fn active_workers() -> usize { ACTIVE_WORKERS.load(Ordering::SeqCst) }

    /// A production reader seam, also suitable for owned Cursor fixtures. The
    /// caller transfers a seekable, known-length source; no reader methods run on
    /// this calling thread. This is not a URL opener or an output/playback API.
    pub fn from_source(
        &mut self,
        source: Box<dyn MediaSource>,
        lease: PlaybackLease,
    ) -> Result<SymphoniaDecoder, String> {
        spawn(Source::Owned(source), lease)
    }
}

impl DecoderFactory for SymphoniaDecoderFactory {
    fn begin(&mut self, track: &Track, lease: PlaybackLease) -> Result<Box<dyn Decoder>, String> {
        let path = track.path.clone();
        let text = path.to_str().ok_or("local audio path must be Unicode")?;
        if !path.is_absolute() || text.len() > 4096 || text.chars().any(char::is_control)
            || text.contains("://") || text.starts_with("\\\\") || text.starts_with("//")
        {
            return Err("choose an absolute local file path; URLs, network and device paths are unsupported".into());
        }
        Ok(Box::new(spawn(Source::Path(path), lease)?))
    }
}

enum Source { Path(PathBuf), Owned(Box<dyn MediaSource>) }
type Reply = Result<DecodedRead, String>;

/// Nonblocking handle. It never joins a worker on release/Drop. Only one request
/// can be in flight, and at most one <=1024-frame reply is retained here. The
/// worker's separately bounded decoded packet is drained before reading another.
pub struct SymphoniaDecoder {
    control: Arc<Control>,
    requests: Option<SyncSender<usize>>,
    replies: Option<Receiver<Reply>>,
    in_flight: bool,
    primed: bool,
    held: Vec<[f32; 2]>,
    offset: usize,
    sample_rate: u32,
    ended: bool,
}

fn spawn(source: Source, lease: PlaybackLease) -> Result<SymphoniaDecoder, String> {
    if lease.is_cancelled() { return Err("decoder lease is cancelled".into()); }
    let slot = WorkerSlot::acquire()?;
    let control = Arc::new(Control {
        alive: AtomicBool::new(true), primed: AtomicBool::new(false), lease,
        wake: Condvar::new(), wait: Mutex::new(()),
        read_remaining: AtomicUsize::new(MAX_PACKET_BYTES),
    });
    let (request_tx, request_rx) = mpsc::sync_channel(1);
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    let worker_control = control.clone();
    thread::Builder::new().name("gamestrike-music-decode".into())
        .spawn(move || {
            let _slot = slot;
            worker(source, worker_control, request_rx, reply_tx);
        })
        .map_err(|error| format!("cannot start decoder worker: {error}"))?;
    Ok(SymphoniaDecoder {
        control, requests: Some(request_tx), replies: Some(reply_rx), in_flight: false,
        primed: false, held: Vec::new(), offset: 0, sample_rate: 0, ended: false,
    })
}

impl Decoder for SymphoniaDecoder {
    fn poll(&mut self, max_frames: usize) -> Reply {
        if max_frames == 0 || max_frames > MAX_BUFFER_FRAMES {
            self.release();
            return Err("decoder request must contain 1..=1024 frames".into());
        }
        if !self.control.live() { self.release(); return Ok(DecodedRead::Pending); }
        if !self.control.authorized(self.primed) { return Ok(DecodedRead::Pending); }
        self.control.wake.notify_all();
        if self.ended { return Ok(DecodedRead::End); }
        if self.held.is_empty() {
            let reply = match self.replies.as_ref().ok_or("decoder is closed")?.try_recv() {
                Ok(reply) => { self.in_flight = false; Some(reply) }
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    self.release();
                    return Err("decoder worker exited without a completion".into());
                }
            };
            if let Some(reply) = reply {
                if !self.control.live() {
                    self.release();
                    return Ok(DecodedRead::Pending);
                }
                match reply {
                    Err(error) => { self.release(); return Err(error); }
                    Ok(DecodedRead::Frames { sample_rate, frames }) => {
                        if frames.is_empty() || frames.len() > MAX_BUFFER_FRAMES {
                            self.release();
                            return Err("decoder worker returned an invalid block".into());
                        }
                        self.sample_rate = sample_rate;
                        self.held = frames;
                        self.offset = 0;
                    }
                    Ok(DecodedRead::End) => {
                        self.ended = true;
                        return Ok(if self.control.authorized(self.primed) {
                            DecodedRead::End
                        } else {
                            DecodedRead::Pending
                        });
                    }
                    Ok(DecodedRead::Pending) => {}
                }
            }
        }
        if !self.control.authorized(self.primed) { return Ok(DecodedRead::Pending); }
        if !self.held.is_empty() {
            let end = (self.offset + max_frames).min(self.held.len());
            let frames = self.held[self.offset..end].to_vec();
            if !self.control.live() {
                self.release();
                return Ok(DecodedRead::Pending);
            }
            if !self.control.authorized(self.primed) { return Ok(DecodedRead::Pending); }
            self.offset = end;
            if end == self.held.len() { self.held.clear(); self.offset = 0; }
            self.primed = true;
            return Ok(DecodedRead::Frames { sample_rate: self.sample_rate, frames });
        }
        if !self.in_flight {
            match self.requests.as_ref().ok_or("decoder is closed")?.try_send(max_frames) {
                Ok(()) => self.in_flight = true,
                Err(TrySendError::Full(_)) => {
                    self.release();
                    return Err("decoder request queue is unexpectedly full".into());
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.release();
                    return Err("decoder worker is unavailable".into());
                }
            }
        }
        Ok(DecodedRead::Pending)
    }

    fn release(&mut self) {
        self.control.close();
        self.requests = None;
        self.replies = None;
        self.held.clear();
        self.offset = 0;
        self.in_flight = false;
    }
}

impl Drop for SymphoniaDecoder {
    fn drop(&mut self) { self.release(); }
}

fn worker(source: Source, control: Arc<Control>, requests: Receiver<usize>, replies: SyncSender<Reply>) {
    let mut source = Some(source);
    let mut engine = None;
    while let Ok(max_frames) = requests.recv() {
        if control.permission().is_err() { break; }
        let reply = (|| {
            if engine.is_none() {
                engine = Some(Engine::open(source.take().ok_or("decoder source is missing")?, control.clone())?);
            }
            engine.as_mut().ok_or("decoder is missing")?.next(max_frames, &control)
        })();
        // A paused worker may retain its completed packet until resume. A revoked
        // worker discards it. No late source completion can revive a released handle.
        if control.permission().is_err() { break; }
        let terminal = matches!(&reply, Err(_) | Ok(DecodedRead::End));
        let primed = matches!(&reply, Ok(DecodedRead::Frames { .. }));
        if replies.try_send(reply).is_err() { break; }
        if primed { control.primed.store(true, Ordering::SeqCst); }
        if terminal { break; }
    }
}

struct GuardedSource {
    source: Box<dyn MediaSource>,
    length: u64,
    control: Arc<Control>,
}

impl GuardedSource {
    fn new(source: Source, control: Arc<Control>) -> Result<Self, String> {
        control.permission().map_err(|error| error.to_string())?;
        let source: Box<dyn MediaSource> = match source {
            Source::Owned(source) => source,
            Source::Path(path) => {
                let metadata = fs::metadata(&path).map_err(|error| format!("cannot inspect local audio file: {error}"))?;
                control.permission().map_err(|error| error.to_string())?;
                if !metadata.is_file() { return Err("selected audio source is not a regular file".into()); }
                let file = File::open(path).map_err(|error| format!("cannot open local audio file: {error}"))?;
                control.permission().map_err(|error| error.to_string())?;
                if !file.metadata().map_err(|error| error.to_string())?.is_file() {
                    return Err("opened audio source is not a regular file".into());
                }
                Box::new(file)
            }
        };
        control.permission().map_err(|error| error.to_string())?;
        let seekable = source.is_seekable();
        control.permission().map_err(|error| error.to_string())?;
        let length = source.byte_len().ok_or("local audio source must have a known length")?;
        control.permission().map_err(|error| error.to_string())?;
        if !seekable { return Err("local audio source must be seekable".into()); }
        Ok(Self { source, length, control })
    }
}

impl Read for GuardedSource {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.control.permission()?;
        if buffer.is_empty() { return Ok(0); }
        let remaining = self.control.read_remaining.load(Ordering::SeqCst);
        if remaining == 0 { return Err(io::Error::other("decoder source-read budget exceeded")); }
        let limit = buffer.len().min(remaining);
        let result = self.source.read(&mut buffer[..limit]);
        self.control.permission()?;
        if let Ok(count) = result {
            self.control.read_remaining.fetch_sub(count, Ordering::SeqCst);
        }
        result
    }
}

impl Seek for GuardedSource {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.control.permission()?;
        let result = self.source.seek(position);
        self.control.permission()?;
        match result {
            Ok(position) if position > self.length => Err(io::Error::other("audio seek exceeds source length")),
            result => result,
        }
    }
}

impl MediaSource for GuardedSource {
    fn is_seekable(&self) -> bool { true }
    fn byte_len(&self) -> Option<u64> { Some(self.length) }
}

fn read_at<const N: usize>(source: &mut GuardedSource, position: u64) -> Result<[u8; N], String> {
    if position.checked_add(N as u64).is_none_or(|end| end > source.length) {
        return Err("audio header is truncated".into());
    }
    source.seek(SeekFrom::Start(position)).map_err(|error| error.to_string())?;
    let mut bytes = [0; N];
    source.read_exact(&mut bytes).map_err(|error| format!("audio header read failed: {error}"))?;
    Ok(bytes)
}

fn preflight(source: &mut GuardedSource) -> Result<(), String> {
    let head = read_at::<12>(source, 0)?;
    if &head[..4] == b"RIFF" && &head[8..] == b"WAVE" {
        let riff_end = u64::from(u32::from_le_bytes(head[4..8].try_into().unwrap())) + 8;
        if riff_end > source.length { return Err("WAV is truncated".into()); }
        let mut position = 12_u64;
        let mut alignment = None;
        for _ in 0..MAX_HEADER_CHUNKS {
            let chunk = read_at::<8>(source, position)?;
            let length = u64::from(u32::from_le_bytes(chunk[4..].try_into().unwrap()));
            let payload = position + 8;
            let end = payload.checked_add(length).ok_or("WAV chunk length overflow")?;
            if end > riff_end { return Err("WAV chunk is truncated".into()); }
            if &chunk[..4] == b"data" {
                let alignment: u64 = alignment.ok_or("WAV data precedes its format")?;
                if length % alignment != 0 { return Err("WAV data contains a partial frame".into()); }
                source.seek(SeekFrom::Start(0)).map_err(|error| error.to_string())?;
                return Ok(());
            }
            if length > MAX_PACKET_BYTES as u64 || end > MAX_PACKET_BYTES as u64 {
                return Err("WAV header/metadata exceeds decoder bound".into());
            }
            if &chunk[..4] == b"fmt " {
                if length < 16 { return Err("WAV format is truncated".into()); }
                let format = read_at::<16>(source, payload)?;
                let tag = u16::from_le_bytes(format[..2].try_into().unwrap());
                let channels = u16::from_le_bytes(format[2..4].try_into().unwrap());
                let rate = u32::from_le_bytes(format[4..8].try_into().unwrap());
                let byte_rate = u32::from_le_bytes(format[8..12].try_into().unwrap());
                let align = u16::from_le_bytes(format[12..14].try_into().unwrap());
                let bits = u16::from_le_bytes(format[14..].try_into().unwrap());
                if !matches!(tag, 1 | 3 | 0xfffe) || !matches!(bits, 8 | 16 | 24 | 32 | 64) {
                    return Err("unsupported WAV encoding".into());
                }
                if !(1..=2).contains(&channels) || rate == 0 || align != channels * (bits / 8)
                    || rate.checked_mul(u32::from(align)) != Some(byte_rate)
                {
                    return Err("invalid WAV rate, channel layout or block alignment".into());
                }
                alignment = Some(u64::from(align));
            }
            position = end + (length & 1);
        }
        return Err("WAV header chunk count exceeds decoder bound".into());
    }
    if &head[..4] == b"fLaC" {
        let mut position = 4_u64;
        for index in 0..MAX_HEADER_CHUNKS {
            let header = read_at::<4>(source, position)?;
            let length = u32::from_be_bytes([0, header[1], header[2], header[3]]) as u64;
            let end = position + 4 + length;
            if end > source.length { return Err("FLAC metadata is truncated".into()); }
            if end > MAX_PACKET_BYTES as u64 { return Err("FLAC metadata exceeds decoder bound".into()); }
            if index == 0 {
                if header[0] & 0x7f != 0 || length != 34 { return Err("FLAC STREAMINFO is invalid".into()); }
                let info = read_at::<34>(source, position + 4)?;
                let maximum = u32::from_be_bytes([0, info[7], info[8], info[9]]) as usize;
                if maximum > MAX_PACKET_BYTES { return Err("FLAC encoded frame exceeds decoder bound".into()); }
            }
            position = end;
            if header[0] & 0x80 != 0 {
                source.seek(SeekFrom::Start(0)).map_err(|error| error.to_string())?;
                return Ok(());
            }
        }
        return Err("FLAC metadata count exceeds decoder bound".into());
    }
    if &head[..3] == b"ID3" {
        if head[6..10].iter().any(|byte| byte & 0x80 != 0) { return Err("invalid ID3 length".into()); }
        let length = head[6..10].iter().fold(0_u64, |length, byte| (length << 7) | u64::from(*byte)) + 10;
        if length > MAX_PACKET_BYTES as u64 || length > source.length {
            return Err("ID3 metadata is oversized or truncated".into());
        }
    } else if head[0] != 0xff || head[1] & 0xe0 != 0xe0 {
        return Err("unsupported local audio container; expected WAV, FLAC or MP3".into());
    }
    source.seek(SeekFrom::Start(0)).map_err(|error| error.to_string())?;
    Ok(())
}

fn validate_parameters(parameters: &CodecParameters) -> Result<(u32, Channels), String> {
    let rate = parameters.sample_rate.filter(|rate| *rate > 0).ok_or("unknown source sample rate")?;
    let channels = parameters.channels.ok_or("unknown source channel layout")?;
    if channels.count() == 0 || channels.count() > 2
        || (channels.count() == 2 && channels != (Channels::FRONT_LEFT | Channels::FRONT_RIGHT))
    {
        return Err("only mono or front-left/front-right stereo is supported".into());
    }
    if parameters.max_frames_per_packet.is_some_and(|frames| frames == 0 || frames > MAX_DECODED_FRAMES as u64)
        || parameters.frames_per_block.is_some_and(|frames| frames > MAX_DECODED_FRAMES as u64)
        || parameters.extra_data.as_ref().is_some_and(|data| data.len() > MAX_PACKET_BYTES)
    {
        return Err("codec parameters exceed decoder staging bounds".into());
    }
    Ok((rate, channels))
}

fn failure(stage: &str, error: SymphoniaError) -> String {
    match error {
        SymphoniaError::ResetRequired => format!("{stage}: stream reset/rate changes are unsupported"),
        error => format!("{stage}: {error}"),
    }
}

struct Engine {
    format: Option<Box<dyn FormatReader>>,
    decoder: Box<dyn CodecDecoder>,
    track_id: u32,
    rate: u32,
    channels: Channels,
    expected_frames: Option<u64>,
    reliable_completion: bool,
    source_byte_len: u64,
    decoded_packet_bytes: u64,
    decoded_frames: u64,
    staging: Option<SampleBuffer<f32>>,
    offset: usize,
}

impl Engine {
    fn open(source: Source, control: Arc<Control>) -> Result<Self, String> {
        let mut source = GuardedSource::new(source, control.clone())?;
        preflight(&mut source)?;
        let source_byte_len = source.length;
        control.read_remaining.store(MAX_PACKET_BYTES, Ordering::SeqCst);
        let stream = MediaSourceStream::new(Box::new(source), MediaSourceStreamOptions {
            buffer_len: SOURCE_BUFFER_BYTES,
        });
        let metadata = MetadataOptions {
            limit_metadata_bytes: Limit::Maximum(64 * 1024), limit_visual_bytes: Limit::Maximum(0),
        };
        let probed = symphonia::default::get_probe()
            .format(&Hint::new(), stream, &FormatOptions {
                enable_gapless: false,
                ..FormatOptions::default()
            }, &metadata)
            .map_err(|error| failure("probe", error))?;
        control.permission().map_err(|error| error.to_string())?;
        let format = probed.format;
        if format.tracks().len() != 1 { return Err("multiple audio tracks are unsupported".into()); }
        let track = format.default_track().ok_or("source has no audio track")?;
        let track_id = track.id;
        if track.codec_params.codec == codecs::CODEC_TYPE_NULL { return Err("source codec is unsupported".into()); }
        validate_parameters(&track.codec_params)?;
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions { verify: true })
            .map_err(|error| failure("codec", error))?;
        let (rate, channels) = validate_parameters(decoder.codec_params())?;
        let expected_frames = decoder.codec_params().n_frames;
        let reliable_completion = decoder.codec_params().codec != codecs::CODEC_TYPE_MP3;
        Ok(Self { format: Some(format), decoder, track_id, rate, channels, expected_frames,
            reliable_completion, source_byte_len, decoded_packet_bytes: 0,
            decoded_frames: 0, staging: None, offset: 0 })
    }

    fn next(&mut self, max_frames: usize, control: &Control) -> Reply {
        if self.staging.is_none() {
            control.permission().map_err(|error| error.to_string())?;
            control.read_remaining.store(MAX_PACKET_BYTES + SOURCE_BUFFER_BYTES, Ordering::SeqCst);
            let packet = match self.format.as_mut().ok_or("audio demuxer is closed")?.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(error)) if error.kind() == io::ErrorKind::UnexpectedEof => {
                    // Pinned MpaReader::next_packet returns complete, forward,
                    // disjoint frame bytes. Resync/tag/info bytes are discarded.
                    // Equal byte coverage therefore excludes omitted/truncated
                    // bytes without implementing another MPEG header parser.
                    if !self.reliable_completion {
                        let source = self.format.take().ok_or("audio demuxer is closed")?.into_inner();
                        if self.decoded_frames == 0
                            || self.decoded_packet_bytes != self.source_byte_len
                            || source.pos() != self.source_byte_len
                            || source.byte_len() != Some(self.source_byte_len)
                        {
                            return Err("MP3 byte coverage is incomplete: skipped metadata, junk or a truncated packet cannot certify clean EOF".into());
                        }
                    } else if self.expected_frames != Some(self.decoded_frames) {
                        return Err("source ended early or its completion could not be verified".into());
                    }
                    if self.decoder.finalize().verify_ok == Some(false) {
                        return Err("decoded source verification failed".into());
                    }
                    return Ok(DecodedRead::End);
                }
                Err(error) => return Err(failure("packet", error)),
            };
            control.permission().map_err(|error| error.to_string())?;
            let duration = packet.dur.checked_add(u64::from(packet.trim_start))
                .and_then(|duration| duration.checked_add(u64::from(packet.trim_end)))
                .ok_or("packet frame count overflow")?;
            if packet.track_id() != self.track_id || packet.buf().is_empty()
                || packet.buf().len() > MAX_PACKET_BYTES || duration == 0
                || duration > MAX_DECODED_FRAMES as u64
            {
                return Err("encoded packet exceeds decoder bounds or uses another track".into());
            }
            let covered_bytes = self.decoded_packet_bytes.checked_add(packet.buf().len() as u64)
                .ok_or("decoded packet byte count overflow")?;
            if !self.reliable_completion && covered_bytes > self.source_byte_len {
                return Err("MP3 packet byte coverage exceeds the source length".into());
            }
            let decoded = self.decoder.decode(&packet).map_err(|error| failure("decode", error))?;
            let spec = *decoded.spec();
            let count = decoded.frames();
            if spec.rate != self.rate || spec.channels != self.channels {
                return Err("decoded sample rate or channel layout changed".into());
            }
            if count == 0 || count > MAX_DECODED_FRAMES || decoded.capacity() > MAX_DECODED_FRAMES {
                return Err("decoded packet exceeds frame staging bound".into());
            }
            let total = self.decoded_frames.checked_add(count as u64).ok_or("decoded frame count overflow")?;
            // MP3 n_frames may be a bitrate estimate. It cannot bound decoded
            // PCM or trim packets; exact byte coverage decides its completion.
            if self.reliable_completion && self.expected_frames.is_some_and(|expected| total > expected) {
                return Err("decoded frames exceed declared source length".into());
            }
            // Bound checks precede this adapter's conversion allocation. Symphonia's
            // own codec/demux buffers are third-party internals, not a sandbox claim.
            let mut samples = SampleBuffer::<f32>::new(count as u64, spec);
            samples.copy_interleaved_ref(decoded);
            if samples.samples().iter().any(|sample| !sample.is_finite()) {
                return Err("decoded PCM contains nonfinite samples".into());
            }
            self.decoded_frames = total;
            self.decoded_packet_bytes = covered_bytes;
            self.staging = Some(samples);
            self.offset = 0;
        }
        control.permission().map_err(|error| error.to_string())?;
        let samples = self.staging.as_ref().ok_or("decoded staging is missing")?.samples();
        let channels = self.channels.count();
        let end = (self.offset + max_frames).min(samples.len() / channels);
        let mut frames = Vec::with_capacity(end - self.offset);
        for frame in samples[self.offset * channels..end * channels].chunks_exact(channels) {
            frames.push([frame[0], if channels == 1 { frame[0] } else { frame[1] }]);
        }
        self.offset = end;
        if end == samples.len() / channels { self.staging = None; self.offset = 0; }
        Ok(DecodedRead::Frames { sample_rate: self.rate, frames })
    }
}
