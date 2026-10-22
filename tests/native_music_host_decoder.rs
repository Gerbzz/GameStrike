//! Fixtures own their WAV bytes; no user files, output devices or network are used.
#![cfg(all(windows, feature = "native-music-host"))]
use gamestrike::music::{
    Track,
    host::decoder::{
        MAX_DECODER_WORKERS, MAX_PACKET_BYTES, SymphoniaDecoder, SymphoniaDecoderFactory,
    },
    playback::{BackendEvent, PlaybackBackend, PlaybackCommand, PlaybackController, PlaybackLease},
    streaming::{DecodedRead, Decoder, DecoderFactory, MAX_BUFFER_FRAMES},
};
use std::{
    collections::VecDeque,
    io::{self, Cursor, Read, Seek, SeekFrom},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use symphonia::core::io::MediaSource;
use uuid::Uuid;

static TESTS: Mutex<()> = Mutex::new(());

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "worker did not settle within fixture deadline"
        );
        thread::yield_now();
    }
}

fn serial() -> MutexGuard<'static, ()> {
    let guard = TESTS.lock().unwrap();
    wait_until(|| SymphoniaDecoderFactory::active_workers() == 0);
    guard
}

#[derive(Default)]
struct Authority {
    lease: Option<PlaybackLease>,
    operation: u64,
    events: VecDeque<BackendEvent>,
}

struct Relay(Arc<Mutex<Authority>>);

impl PlaybackBackend for Relay {
    fn load(&mut self, _: &Track, lease: PlaybackLease) -> Result<(), String> {
        self.0.lock().unwrap().lease = Some(lease);
        Ok(())
    }
    fn play(&mut self, _: &PlaybackLease, operation: u64) -> Result<(), String> {
        self.0.lock().unwrap().operation = operation;
        Ok(())
    }
    fn pause(&mut self, _: &PlaybackLease, _: u64) -> Result<(), String> {
        Ok(())
    }
    fn release(&mut self, _: &PlaybackLease) {}
    fn poll(&mut self) -> Option<BackendEvent> {
        self.0.lock().unwrap().events.pop_front()
    }
}

struct Fixture {
    authority: Arc<Mutex<Authority>>,
    controller: PlaybackController,
    track: Track,
    lease: PlaybackLease,
}

impl Fixture {
    fn new() -> Self {
        let authority = Arc::new(Mutex::new(Authority::default()));
        let track = Track {
            id: Uuid::from_u128(1),
            title: "Owned WAV fixture".into(),
            path: std::env::temp_dir().join("gamestrike-owned-decoder-fixture.wav"),
        };
        let mut controller = PlaybackController::new(Box::new(Relay(authority.clone())));
        controller
            .replace_queue(std::slice::from_ref(&track))
            .unwrap();
        controller
            .dispatch(PlaybackCommand::Play(track.id))
            .unwrap();
        let lease = authority.lock().unwrap().lease.clone().unwrap();
        Self {
            authority,
            controller,
            track,
            lease,
        }
    }

    // The relay supplies controller authority only. No output adapter is attached.
    fn activate(&mut self) {
        self.authority
            .lock()
            .unwrap()
            .events
            .push_back(BackendEvent::Ready {
                generation: self.lease.generation(),
            });
        self.controller.poll();
        let operation = self.authority.lock().unwrap().operation;
        self.authority
            .lock()
            .unwrap()
            .events
            .push_back(BackendEvent::Started {
                generation: self.lease.generation(),
                operation,
            });
        self.controller.poll();
        assert!(self.lease.may_output());
    }

    fn decoder(&self, bytes: Vec<u8>) -> SymphoniaDecoder {
        SymphoniaDecoderFactory::new()
            .from_source(Box::new(Cursor::new(bytes)), self.lease.clone())
            .unwrap()
    }
}

fn next(decoder: &mut SymphoniaDecoder, max_frames: usize) -> Result<DecodedRead, String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let read = decoder.poll(max_frames)?;
        if !matches!(read, DecodedRead::Pending) {
            return Ok(read);
        }
        assert!(
            Instant::now() < deadline,
            "no decoder reply within fixture deadline"
        );
        thread::yield_now();
    }
}

fn wav(format: u16, bits: u16, channels: u16, rate: u32, data: &[u8]) -> Vec<u8> {
    let align = channels * (bits / 8);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36_u32 + data.len() as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&format.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * u32::from(align)).to_le_bytes());
    bytes.extend_from_slice(&align.to_le_bytes());
    bytes.extend_from_slice(&bits.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
    bytes.extend_from_slice(data);
    bytes
}

fn pcm16(channels: u16, samples: &[i16]) -> Vec<u8> {
    let data: Vec<u8> = samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();
    wav(1, 16, channels, 44_100, &data)
}

fn normalized(sample: i16) -> f32 {
    f32::from(sample) / 32768.0
}

// Original owned MPEG-1 Layer III silence: 128 kb/s, 44.1 kHz stereo, no CRC,
// zero side information (no coded spectral coefficients) and zero main data.
// This fixture is intentionally decoded by Symphonia, not a replacement codec.
fn mp3_silence() -> Vec<u8> {
    let mut frame = vec![0_u8; 417];
    frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0x00]);
    frame.repeat(3)
}

#[test]
fn factory_is_idle_and_rejects_nonlocal_paths_without_starting_a_worker() {
    let _guard = serial();
    let fixture = Fixture::new();
    let mut factory = SymphoniaDecoderFactory::new();
    assert_eq!(SymphoniaDecoderFactory::active_workers(), 0);
    for path in [
        "relative.wav",
        "https://example.test/music.wav",
        "//server/share/music.wav",
    ] {
        let mut track = fixture.track.clone();
        track.path = path.into();
        assert!(factory.begin(&track, fixture.lease.clone()).is_err());
        assert_eq!(SymphoniaDecoderFactory::active_workers(), 0);
    }
}

#[test]
fn real_wav_decoder_preserves_signed_stereo_sample_order_and_source_rate() {
    let _guard = serial();
    let mut fixture = Fixture::new();
    let samples = [-32768, 32767, -16384, 8192, 0, -8192];
    let mut decoder = fixture.decoder(pcm16(2, &samples));
    let first = next(&mut decoder, 2).unwrap();
    assert!(
        matches!(&first, DecodedRead::Frames { sample_rate: 44_100, frames }
        if frames == &vec![[normalized(-32768), normalized(32767)],
                          [normalized(-16384), normalized(8192)]])
    );
    assert!(!fixture.lease.may_output()); // Decoding did not enable playback authority.
    fixture.activate();
    assert_eq!(
        next(&mut decoder, 2).unwrap(),
        DecodedRead::Frames {
            sample_rate: 44_100,
            frames: vec![[0.0, normalized(-8192)]],
        }
    );
    assert_eq!(next(&mut decoder, 2).unwrap(), DecodedRead::End);
    decoder.release();
}

#[test]
fn actual_mono_pcm_is_explicitly_duplicated_into_both_channels() {
    let _guard = serial();
    let mut fixture = Fixture::new();
    let samples = [-32768, -1234, 0, 2345, 32767];
    let mut decoder = fixture.decoder(pcm16(1, &samples));
    let first = next(&mut decoder, MAX_BUFFER_FRAMES).unwrap();
    let expected: Vec<[f32; 2]> = samples
        .iter()
        .map(|sample| [normalized(*sample); 2])
        .collect();
    assert_eq!(
        first,
        DecodedRead::Frames {
            sample_rate: 44_100,
            frames: expected
        }
    );
    fixture.activate();
    assert_eq!(
        next(&mut decoder, MAX_BUFFER_FRAMES).unwrap(),
        DecodedRead::End
    );
    decoder.release();
}

#[test]
fn raw_mp3_pcm_completes_only_after_all_complete_packet_bytes() {
    let _guard = serial();
    let mut fixture = Fixture::new();
    let mut decoder = fixture.decoder(mp3_silence());
    let mut samples = 0;
    loop {
        match next(&mut decoder, MAX_BUFFER_FRAMES).unwrap() {
            DecodedRead::Frames {
                sample_rate,
                frames,
            } => {
                assert_eq!(sample_rate, 44_100);
                assert!(frames.iter().all(|frame| *frame == [0.0, 0.0]));
                samples += frames.len();
                if !fixture.lease.may_output() {
                    fixture.activate();
                }
            }
            DecodedRead::End => break,
            DecodedRead::Pending => unreachable!(),
        }
    }
    assert_eq!(samples, 3 * 1152);
    decoder.release();
}

#[test]
fn long_wav_keeps_exact_order_across_packets_and_different_request_sizes() {
    let _guard = serial();
    let mut fixture = Fixture::new();
    let expected: Vec<[f32; 2]> = (0..5000_i16)
        .map(|sample| [normalized(sample), normalized(-sample)])
        .collect();
    let samples: Vec<i16> = (0..5000_i16).flat_map(|sample| [sample, -sample]).collect();
    let mut decoder = fixture.decoder(pcm16(2, &samples));
    let mut actual = Vec::new();
    let sizes = [7, 1024, 3, 257];
    for index in 0..10_000 {
        let limit = sizes[index % sizes.len()];
        match next(&mut decoder, limit).unwrap() {
            DecodedRead::Frames {
                sample_rate,
                frames,
            } => {
                assert_eq!(sample_rate, 44_100);
                assert!(!frames.is_empty() && frames.len() <= limit);
                actual.extend(frames);
                if !fixture.lease.may_output() {
                    fixture.activate();
                }
            }
            DecodedRead::End => break,
            DecodedRead::Pending => unreachable!(),
        }
    }
    assert_eq!(actual, expected);
    decoder.release();
}

#[test]
fn a_smaller_poll_limit_splits_an_already_requested_reply_without_losing_samples() {
    let _guard = serial();
    let mut fixture = Fixture::new();
    let mut decoder = fixture.decoder(pcm16(1, &[1024, 2048, 3072, 4096]));
    assert_eq!(decoder.poll(4).unwrap(), DecodedRead::Pending);
    assert_eq!(
        next(&mut decoder, 1).unwrap(),
        DecodedRead::Frames {
            sample_rate: 44_100,
            frames: vec![[normalized(1024); 2]],
        }
    );
    fixture.activate();
    assert_eq!(
        next(&mut decoder, 3).unwrap(),
        DecodedRead::Frames {
            sample_rate: 44_100,
            frames: vec![
                [normalized(2048); 2],
                [normalized(3072); 2],
                [normalized(4096); 2]
            ],
        }
    );
    decoder.release();
}

#[test]
fn truncation_unsupported_data_invalid_rate_and_multichannel_are_errors_not_eof() {
    let _guard = serial();
    let mut truncated = pcm16(2, &[1, 2, 3, 4]);
    truncated.truncate(truncated.len() - 2);
    let fixtures = [
        truncated,
        b"not a supported audio container".to_vec(),
        wav(1, 16, 1, 0, &[0, 0]),
        pcm16(3, &[1, 2, 3]),
        wav(0x7777, 16, 1, 44_100, &[0, 0]),
    ];
    for bytes in fixtures {
        let fixture = Fixture::new();
        let mut decoder = fixture.decoder(bytes);
        assert!(next(&mut decoder, MAX_BUFFER_FRAMES).is_err());
        decoder.release();
        wait_until(|| SymphoniaDecoderFactory::active_workers() == 0);
    }
}

#[test]
fn malformed_alignment_and_nonfinite_float_pcm_fail_closed() {
    let _guard = serial();
    let mut bad_align = pcm16(2, &[1, 2]);
    bad_align[32..34].copy_from_slice(&u16::MAX.to_le_bytes());
    for bytes in [bad_align, wav(3, 32, 1, 44_100, &f32::NAN.to_le_bytes())] {
        let fixture = Fixture::new();
        let mut decoder = fixture.decoder(bytes);
        assert!(next(&mut decoder, MAX_BUFFER_FRAMES).is_err());
        decoder.release();
        wait_until(|| SymphoniaDecoderFactory::active_workers() == 0);
    }
}

#[test]
fn inflated_flac_frame_and_wav_metadata_sizes_are_rejected_before_decoding() {
    let _guard = serial();
    let mut info = [0_u8; 34];
    let maximum = (MAX_PACKET_BYTES as u32 + 1).to_be_bytes();
    info[7..10].copy_from_slice(&maximum[1..]);
    let mut flac = b"fLaC\x80\0\0\x22".to_vec();
    flac.extend_from_slice(&info);

    let length = MAX_PACKET_BYTES + 1;
    let tail = pcm16(1, &[1]);
    let mut wave = b"RIFF".to_vec();
    wave.extend_from_slice(
        &(4_u32 + 8 + length as u32 + 1 + (tail.len() as u32 - 12)).to_le_bytes(),
    );
    wave.extend_from_slice(b"WAVEJUNK");
    wave.extend_from_slice(&(length as u32).to_le_bytes());
    wave.resize(wave.len() + length + 1, 0);
    wave.extend_from_slice(&tail[12..]);
    for bytes in [flac, wave] {
        let fixture = Fixture::new();
        let mut decoder = fixture.decoder(bytes);
        let result = next(&mut decoder, MAX_BUFFER_FRAMES);
        assert!(matches!(result, Err(error) if error.contains("bound")));
        decoder.release();
        wait_until(|| SymphoniaDecoderFactory::active_workers() == 0);
    }
}

struct CountedSource {
    cursor: Cursor<Vec<u8>>,
    reads: Arc<AtomicUsize>,
}

impl Read for CountedSource {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.cursor.read(buffer)
    }
}
impl Seek for CountedSource {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.cursor.seek(position)
    }
}
impl MediaSource for CountedSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.cursor.get_ref().len() as u64)
    }
}

#[test]
fn invalid_poll_bounds_and_release_before_first_poll_do_no_source_reads() {
    let _guard = serial();
    for limit in [0, MAX_BUFFER_FRAMES + 1] {
        let fixture = Fixture::new();
        let reads = Arc::new(AtomicUsize::new(0));
        let source = CountedSource {
            cursor: Cursor::new(pcm16(1, &[1])),
            reads: reads.clone(),
        };
        let mut decoder = SymphoniaDecoderFactory::new()
            .from_source(Box::new(source), fixture.lease.clone())
            .unwrap();
        assert!(decoder.poll(limit).is_err());
        decoder.release();
        wait_until(|| SymphoniaDecoderFactory::active_workers() == 0);
        assert_eq!(reads.load(Ordering::SeqCst), 0);
    }
    let fixture = Fixture::new();
    let reads = Arc::new(AtomicUsize::new(0));
    let source = CountedSource {
        cursor: Cursor::new(pcm16(1, &[1])),
        reads: reads.clone(),
    };
    let mut decoder = SymphoniaDecoderFactory::new()
        .from_source(Box::new(source), fixture.lease.clone())
        .unwrap();
    decoder.release();
    assert_eq!(decoder.poll(1).unwrap(), DecodedRead::Pending);
    wait_until(|| SymphoniaDecoderFactory::active_workers() == 0);
    assert_eq!(reads.load(Ordering::SeqCst), 0);
}

#[test]
fn pause_stops_source_work_after_priming_and_resume_keeps_the_next_frame() {
    let _guard = serial();
    let mut fixture = Fixture::new();
    let reads = Arc::new(AtomicUsize::new(0));
    let source = CountedSource {
        cursor: Cursor::new(pcm16(1, &[1000, 2000, 3000, 4000])),
        reads: reads.clone(),
    };
    let mut decoder = SymphoniaDecoderFactory::new()
        .from_source(Box::new(source), fixture.lease.clone())
        .unwrap();
    assert!(matches!(
        next(&mut decoder, 1).unwrap(),
        DecodedRead::Frames { .. }
    ));
    let before = reads.load(Ordering::SeqCst);
    for _ in 0..20 {
        assert_eq!(decoder.poll(1).unwrap(), DecodedRead::Pending);
    }
    assert_eq!(reads.load(Ordering::SeqCst), before);
    fixture.activate();
    fixture.controller.dispatch(PlaybackCommand::Pause).unwrap();
    for _ in 0..20 {
        assert_eq!(decoder.poll(1).unwrap(), DecodedRead::Pending);
    }
    assert_eq!(reads.load(Ordering::SeqCst), before);
    fixture
        .controller
        .dispatch(PlaybackCommand::Play(fixture.track.id))
        .unwrap();
    assert_eq!(
        next(&mut decoder, 1).unwrap(),
        DecodedRead::Frames {
            sample_rate: 44_100,
            frames: vec![[normalized(2000); 2]],
        }
    );
    decoder.release();
}

#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    wake: Condvar,
    entered: AtomicBool,
}

impl Gate {
    fn open(&self) {
        *self.open.lock().unwrap() = true;
        self.wake.notify_all();
    }
}

struct BlockedSource {
    cursor: Cursor<Vec<u8>>,
    gate: Arc<Gate>,
}

impl Read for BlockedSource {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.gate.entered.store(true, Ordering::SeqCst);
        let mut open = self.gate.open.lock().unwrap();
        while !*open {
            open = self.gate.wake.wait(open).unwrap();
        }
        self.cursor.read(buffer)
    }
}
impl Seek for BlockedSource {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.cursor.seek(position)
    }
}
impl MediaSource for BlockedSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.cursor.get_ref().len() as u64)
    }
}

#[test]
fn blocked_reads_do_not_block_poll_or_release_and_keep_the_global_worker_budget() {
    let _guard = serial();
    let fixture = Fixture::new();
    let mut factory = SymphoniaDecoderFactory::new();
    let mut decoders = Vec::new();
    let mut gates = Vec::new();
    for _ in 0..MAX_DECODER_WORKERS {
        let gate = Arc::new(Gate::default());
        let source = BlockedSource {
            cursor: Cursor::new(pcm16(1, &[1, 2])),
            gate: gate.clone(),
        };
        let mut decoder = factory
            .from_source(Box::new(source), fixture.lease.clone())
            .unwrap();
        assert_eq!(decoder.poll(1).unwrap(), DecodedRead::Pending);
        wait_until(|| gate.entered.load(Ordering::SeqCst));
        decoder.release(); // Deliberately leave the owned reader blocked after release.
        decoders.push(decoder);
        gates.push(gate);
    }
    assert_eq!(
        SymphoniaDecoderFactory::active_workers(),
        MAX_DECODER_WORKERS
    );
    let refused = factory.from_source(Box::new(Cursor::new(pcm16(1, &[3]))), fixture.lease.clone());
    assert!(refused.is_err());
    drop(decoders);
    for gate in gates {
        gate.open();
    }
    wait_until(|| SymphoniaDecoderFactory::active_workers() == 0);
}

#[test]
fn revoked_lease_discards_late_reader_completion_and_never_publishes_pcm() {
    let _guard = serial();
    let mut fixture = Fixture::new();
    let gate = Arc::new(Gate::default());
    let source = BlockedSource {
        cursor: Cursor::new(pcm16(1, &[1, 2])),
        gate: gate.clone(),
    };
    let mut decoder = SymphoniaDecoderFactory::new()
        .from_source(Box::new(source), fixture.lease.clone())
        .unwrap();
    assert_eq!(decoder.poll(1).unwrap(), DecodedRead::Pending);
    wait_until(|| gate.entered.load(Ordering::SeqCst));
    fixture.controller.dispatch(PlaybackCommand::Stop).unwrap();
    gate.open();
    wait_until(|| SymphoniaDecoderFactory::active_workers() == 0);
    assert_eq!(decoder.poll(1).unwrap(), DecodedRead::Pending);
    decoder.release();
}
