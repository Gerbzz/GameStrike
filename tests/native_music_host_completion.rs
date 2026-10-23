//! Original owned MPEG frames only; no media files, devices, output or network.
#![cfg(all(windows, feature = "native-music-host"))]
use gamestrike::music::{
    Track,
    host::decoder::{SymphoniaDecoder, SymphoniaDecoderFactory},
    playback::{BackendEvent, PlaybackBackend, PlaybackCommand, PlaybackController, PlaybackLease},
    streaming::{DecodedRead, Decoder, MAX_BUFFER_FRAMES},
};
use std::{
    collections::VecDeque,
    io::Cursor,
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

static TESTS: Mutex<()> = Mutex::new(());

fn wait_workers() {
    let deadline = Instant::now() + Duration::from_secs(5);
    while SymphoniaDecoderFactory::active_workers() != 0 {
        assert!(
            Instant::now() < deadline,
            "previous fixture worker did not exit"
        );
        thread::yield_now();
    }
}

fn serial() -> MutexGuard<'static, ()> {
    // This mutex carries no fixture data. Keep serialization after an earlier
    // test panic so its real failure does not mask independent decoder results.
    let guard = TESTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    wait_workers();
    guard
}

#[derive(Default)]
struct Authority {
    lease: Option<PlaybackLease>,
    events: VecDeque<BackendEvent>,
}
struct Relay(Arc<Mutex<Authority>>);

impl PlaybackBackend for Relay {
    fn load(&mut self, _: &Track, lease: PlaybackLease) -> Result<(), String> {
        self.0.lock().unwrap().lease = Some(lease);
        Ok(())
    }
    fn play(&mut self, _: &PlaybackLease, _: u64) -> Result<(), String> {
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
    lease: PlaybackLease,
}

impl Fixture {
    fn new() -> Self {
        let authority = Arc::new(Mutex::new(Authority::default()));
        let track = Track {
            id: Uuid::from_u128(1),
            title: "Owned MP3 completion fixture".into(),
            path: std::env::temp_dir().join("gamestrike-owned-completion-fixture.mp3"),
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
            lease,
        }
    }

    fn activate(&mut self) {
        self.authority
            .lock()
            .unwrap()
            .events
            .push_back(BackendEvent::Ready {
                generation: self.lease.generation(),
            });
        self.controller.poll();
        assert!(self.lease.may_output()); // Authority relay only; there is no output adapter.
    }

    fn decoder(&self, bytes: Vec<u8>) -> SymphoniaDecoder {
        SymphoniaDecoderFactory::new()
            .from_source(Box::new(Cursor::new(bytes)), self.lease.clone())
            .unwrap()
    }

    fn collect(&mut self, decoder: &mut SymphoniaDecoder) -> (Vec<[f32; 2]>, Result<(), String>) {
        let mut pcm = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(
                Instant::now() < deadline,
                "fixture decoder did not complete"
            );
            match decoder.poll(MAX_BUFFER_FRAMES) {
                Ok(DecodedRead::Frames {
                    sample_rate,
                    frames,
                }) => {
                    assert_eq!(sample_rate, 44_100);
                    assert!(!frames.is_empty() && frames.len() <= MAX_BUFFER_FRAMES);
                    assert!(frames.iter().all(|frame| *frame == [0.0, 0.0]));
                    pcm.extend(frames);
                    if !self.lease.may_output() {
                        self.activate();
                    }
                }
                Ok(DecodedRead::Pending) => thread::yield_now(),
                Ok(DecodedRead::End) => return (pcm, Ok(())),
                Err(error) => return (pcm, Err(error)),
            }
        }
    }
}

// MPEG-1 Layer III, 44.1 kHz stereo, no CRC, zero side information and zero
// spectral coefficients. This builds original fixture bytes, not a media parser.
fn frame(bitrate_index: u8, padding: bool) -> Vec<u8> {
    let rates: [usize; 15] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
    ];
    let length = 144 * rates[bitrate_index as usize] * 1000 / 44_100 + if padding { 1 } else { 0 };
    let mut bytes = vec![0; length];
    bytes[..4].copy_from_slice(&[
        0xff,
        0xfb,
        (bitrate_index << 4) | (u8::from(padding) << 1),
        0,
    ]);
    bytes
}

fn raw_mp3() -> Vec<u8> {
    frame(9, false).repeat(3)
}

#[test]
fn complete_raw_mp3_decodes_pcm_and_ends_at_an_exact_packet_byte_boundary() {
    let _guard = serial();
    let mut fixture = Fixture::new();
    let mut decoder = fixture.decoder(raw_mp3());
    let (pcm, result) = fixture.collect(&mut decoder);
    assert_eq!(result, Ok(()));
    assert_eq!(pcm.len(), 3 * 1152);
    assert_eq!(decoder.poll(MAX_BUFFER_FRAMES).unwrap(), DecodedRead::End);
    decoder.release();
}

#[test]
fn long_variable_bitrate_and_padding_ignore_the_inaccurate_duration_estimate() {
    let _guard = serial();
    let mut bytes = Vec::new();
    // The demuxer's >16-frame estimate samples these large frames first. Most
    // actual frames are much shorter, so its estimated n_frames is too small.
    for index in 0..80 {
        bytes.extend(frame(if index < 20 { 14 } else { 1 }, index % 2 != 0));
    }
    let mut fixture = Fixture::new();
    let mut decoder = fixture.decoder(bytes);
    let (pcm, result) = fixture.collect(&mut decoder);
    assert_eq!(result, Ok(()));
    assert_eq!(pcm.len(), 80 * 1152);
    decoder.release();
}

#[test]
fn trailing_partial_headers_never_become_clean_eof() {
    let _guard = serial();
    for count in 1..=3 {
        let mut bytes = raw_mp3();
        bytes.extend_from_slice(&frame(9, false)[..count]);
        let mut fixture = Fixture::new();
        let mut decoder = fixture.decoder(bytes);
        let (pcm, result) = fixture.collect(&mut decoder);
        assert_eq!(pcm.len(), 3 * 1152);
        assert!(matches!(result, Err(error) if error.contains("byte coverage")));
        decoder.release();
        wait_workers();
    }
}

#[test]
fn a_truncated_packet_body_is_not_hidden_by_the_frame_count_estimate() {
    let _guard = serial();
    for removed in [1, 200] {
        let mut bytes = raw_mp3();
        bytes.truncate(bytes.len() - removed);
        let mut fixture = Fixture::new();
        let mut decoder = fixture.decoder(bytes);
        let (pcm, result) = fixture.collect(&mut decoder);
        assert_eq!(pcm.len(), 2 * 1152);
        assert!(matches!(result, Err(error) if error.contains("byte coverage")));
        decoder.release();
        wait_workers();
    }
}

#[test]
fn skipped_middle_junk_and_trailing_bytes_remain_unverified() {
    let _guard = serial();
    let mut middle = frame(9, false).repeat(2);
    middle.extend_from_slice(&[0x12, 0x34, 0x56, 0x78, 0x90]);
    middle.extend(frame(9, false));
    let mut trailing = raw_mp3();
    trailing.extend_from_slice(&[0x12, 0x34, 0x56, 0x78, 0x90]);
    for bytes in [middle, trailing] {
        let mut fixture = Fixture::new();
        let mut decoder = fixture.decoder(bytes);
        let (pcm, result) = fixture.collect(&mut decoder);
        assert_eq!(pcm.len(), 3 * 1152);
        assert!(matches!(result, Err(error) if error.contains("byte coverage")));
        decoder.release();
        wait_workers();
    }
}

#[test]
fn skipped_id3_prefix_and_xing_info_frames_do_not_receive_false_completion() {
    let _guard = serial();
    // ID3v2.4: a 13-byte tag body containing one complete TIT2 frame. The
    // frame has a 3-byte UTF-8 body (encoding byte, title "A", terminator).
    // A zero-byte tag body errors inside this version's metadata reader before
    // any audio is reached, so it cannot exercise skipped-byte completion.
    let mut tagged = b"ID3\x04\0\0\0\0\0\x0d".to_vec();
    tagged.extend_from_slice(b"TIT2\0\0\0\x03\0\0\x03A\0");
    tagged.extend(raw_mp3());
    let mut info = frame(9, false);
    info[36..40].copy_from_slice(b"Xing"); // Side information ends at byte 36.
    info.extend(raw_mp3());
    for (name, bytes) in [("ID3 title prefix", tagged), ("Xing info frame", info)] {
        let mut fixture = Fixture::new();
        let mut decoder = fixture.decoder(bytes);
        let (pcm, result) = fixture.collect(&mut decoder);
        assert_eq!(
            pcm.len(),
            3 * 1152,
            "{name} must decode its audio frames: {result:?}"
        );
        assert!(
            matches!(&result, Err(error) if error.contains("byte coverage")),
            "{name} must fail closed for skipped bytes: {result:?}"
        );
        decoder.release();
        wait_workers();
    }
}
