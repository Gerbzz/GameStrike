#![forbid(unsafe_code)]
mod common;

use common::assert_close;
use gamestrike_audio_core::*;

fn tone(frames: usize) -> Vec<f32> {
    (0..frames).map(|i| {
        (std::f64::consts::TAU * 440.0 * i as f64 / SAMPLE_RATE_HZ as f64).sin() as f32 * 0.6
    }).collect()
}

fn amplitude(samples: &[f32], hz: f64) -> f64 {
    let (mut real, mut imag) = (0.0, 0.0);
    for (i, &sample) in samples.iter().enumerate() {
        let phase = std::f64::consts::TAU * hz * i as f64 / SAMPLE_RATE_HZ as f64;
        real += sample as f64 * phase.cos();
        imag -= sample as f64 * phase.sin();
    }
    2.0 * real.hypot(imag) / samples.len() as f64
}

fn has_robot_sidebands(samples: &[f32]) -> bool {
    amplitude(samples, 360.0) > 0.25
        && amplitude(samples, 520.0) > 0.25
        && amplitude(samples, 440.0) < 0.02
}

#[test]
fn bypass_preserves_normalized_mono_samples() {
    let input = [-1.0, -0.4, -0.0, 0.0, 0.25, 1.0];
    let mut output = [99.0; 6];
    VoiceProcessor::new().process(&input, VoicePreset::Bypass, SAMPLE_RATE_HZ, &mut output)
        .expect("bypass must process valid audio");
    for (actual, expected) in output.iter().zip(input) {
        assert_eq!(actual.to_bits(), expected.to_bits());
    }
}

#[test]
fn robot_440_hz_voice_has_360_and_520_hz_sidebands() {
    let input = tone(96_000);
    let mut output = vec![0.0; input.len()];
    VoiceProcessor::new().process(&input, VoicePreset::Robot, SAMPLE_RATE_HZ, &mut output)
        .expect("robot must process valid audio");
    assert!(has_robot_sidebands(&output[48_000..]),
        "80 Hz ring modulation must produce both sidebands and suppress the dry carrier");
    assert!(output.iter().all(|s| s.is_finite() && (-1.0..=1.0).contains(s)));
}

#[test]
fn spectral_oracle_rejects_a_dry_only_substitute() {
    let dry_only = tone(48_000);
    assert!(!has_robot_sidebands(&dry_only), "dry-only output must fail the Robot oracle");
    assert!(amplitude(&dry_only, 440.0) > 0.59);
}

#[test]
fn robot_phase_is_continuous_across_irregular_block_boundaries() {
    let input = tone(4_321);
    let mut whole = vec![0.0; input.len()];
    VoiceProcessor::new().process(&input, VoicePreset::Robot, SAMPLE_RATE_HZ, &mut whole).unwrap();
    let mut processor = VoiceProcessor::new();
    let mut split = vec![0.0; input.len()];
    let mut start = 0;
    for size in [1, 17, 223, 31, 997, 3, 1_800, 1_249] {
        let end = start + size;
        processor.process(&input[start..end], VoicePreset::Robot, SAMPLE_RATE_HZ,
            &mut split[start..end]).unwrap();
        start = end;
    }
    assert_eq!(start, input.len());
    assert_close(&split, &whole, 0.000_01);
}

#[test]
fn preset_change_blends_for_five_ms_then_reaches_wet_signal() {
    assert_eq!(PRESET_TRANSITION_FRAMES, SAMPLE_RATE_HZ as usize / 200);
    let mut switched = VoiceProcessor::new();
    let mut reference = VoiceProcessor::new();
    let mut warmup = [0.0; 960];
    switched.process(&[1.0; 960], VoicePreset::Bypass, SAMPLE_RATE_HZ, &mut warmup).unwrap();
    reference.process(&[1.0; 960], VoicePreset::Robot, SAMPLE_RATE_HZ, &mut warmup).unwrap();
    let mut blended = [0.0; 480];
    let mut wet = [0.0; 480];
    switched.process(&[1.0; 480], VoicePreset::Robot, SAMPLE_RATE_HZ, &mut blended).unwrap();
    reference.process(&[1.0; 480], VoicePreset::Robot, SAMPLE_RATE_HZ, &mut wet).unwrap();
    assert!((blended[0] - 1.0).abs() < 0.015, "transition must begin near dry");
    assert!(blended[..120].iter().map(|s| (s - 1.0).abs()).sum::<f32>() > 0.1,
        "transition must actually blend rather than hold dry for five ms");
    for i in 0..PRESET_TRANSITION_FRAMES {
        assert!(blended[i] >= wet[i].min(1.0) - 0.000_01
            && blended[i] <= wet[i].max(1.0) + 0.000_01);
    }
    assert_close(&blended[PRESET_TRANSITION_FRAMES - 1..],
        &wet[PRESET_TRANSITION_FRAMES - 1..], 0.000_01);
    let mut back_to_dry = [0.0; 480];
    switched.process(&[1.0; 480], VoicePreset::Bypass, SAMPLE_RATE_HZ, &mut back_to_dry).unwrap();
    assert_close(&back_to_dry[PRESET_TRANSITION_FRAMES - 1..], &[1.0; 241], 0.000_01);
}

#[test]
fn reset_clears_oscillator_and_transition_history() {
    let mut used = VoiceProcessor::new();
    used.process(&tone(777), VoicePreset::Robot, SAMPLE_RATE_HZ, &mut [0.0; 777]).unwrap();
    used.reset().unwrap();
    let input = tone(1_000);
    let mut reset_output = [0.0; 1_000];
    let mut fresh_output = [0.0; 1_000];
    used.process(&input, VoicePreset::Robot, SAMPLE_RATE_HZ, &mut reset_output).unwrap();
    VoiceProcessor::new().process(&input, VoicePreset::Robot, SAMPLE_RATE_HZ, &mut fresh_output).unwrap();
    assert_close(&reset_output, &fresh_output, 0.000_01);
}

#[test]
fn invalid_voice_samples_fail_closed() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.001, -1.001] {
        let mut output = [0.9; 3];
        let result = VoiceProcessor::new().process(&[0.5, bad, -0.5], VoicePreset::Robot,
            SAMPLE_RATE_HZ, &mut output);
        assert_eq!(result, Err(AudioError::InvalidSample));
        assert_eq!(output, [0.0; 3], "invalid input cannot leave earlier output behind");
    }
}

#[test]
fn wrong_voice_rate_and_length_fail_closed() {
    let mut output = [0.9; 3];
    assert_eq!(VoiceProcessor::new().process(&[0.1; 3], VoicePreset::Bypass, 44_100, &mut output),
        Err(AudioError::InvalidSampleRate));
    assert_eq!(output, [0.0; 3]);
    output.fill(0.9);
    assert_eq!(VoiceProcessor::new().process(&[0.1; 2], VoicePreset::Bypass, SAMPLE_RATE_HZ, &mut output),
        Err(AudioError::LengthMismatch));
    assert_eq!(output, [0.0; 3]);
}

#[test]
fn stereo_clips_downmix_by_half_left_plus_right() {
    let mut output = [99.0; 3];
    downmix_stereo(&[0.8, 0.2, -1.0, 1.0, -0.2, -0.6], SAMPLE_RATE_HZ, &mut output).unwrap();
    assert_close(&output, &[0.5, 0.0, -0.4], 0.000_01);
}

#[test]
fn malformed_stereo_clips_fail_closed() {
    for (input, rate, error) in [
        (vec![0.1, 0.2, 0.3], SAMPLE_RATE_HZ, AudioError::LengthMismatch),
        (vec![0.1; 4], 44_100, AudioError::InvalidSampleRate),
        (vec![0.1, f32::NAN, 0.2, 0.3], SAMPLE_RATE_HZ, AudioError::InvalidSample),
    ] {
        let mut output = [0.9; 2];
        assert_eq!(downmix_stereo(&input, rate, &mut output), Err(error));
        assert_eq!(output, [0.0; 2]);
    }
}
