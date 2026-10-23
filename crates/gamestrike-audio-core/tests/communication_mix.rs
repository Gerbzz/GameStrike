#![forbid(unsafe_code)]
mod common;

use common::{assert_close, unity};
use gamestrike_audio_core::*;

fn policy(voice: VoiceState, ptt_open: bool, tx: TxState) -> CommPolicy {
    CommPolicy {
        voice,
        ptt_open,
        tx,
    }
}

#[test]
fn communication_bus_combines_only_processed_voice_and_owned_clips() {
    let mut output = [0.0; 3];
    process_communication(
        &[0.8, -0.4, 0.0],
        &[0.4, 0.4, -0.8],
        CommGains {
            voice: 0.5,
            clips: 0.25,
        },
        policy(VoiceState::Enabled, true, TxState::Ready),
        &mut output,
    )
    .unwrap();
    assert_close(&output, &[0.5, -0.1, -0.2], 0.000_01);
}

#[test]
fn ptt_gates_voice_while_owned_clips_keep_playing() {
    let mut output = [0.0; 2];
    process_communication(
        &[0.8, -0.8],
        &[0.2, -0.3],
        unity(),
        policy(VoiceState::Enabled, false, TxState::Ready),
        &mut output,
    )
    .unwrap();
    assert_eq!(output, [0.2, -0.3]);
}

#[test]
fn voice_disabled_or_muted_does_not_mute_owned_clips() {
    for voice in [VoiceState::Disabled, VoiceState::Muted] {
        let mut output = [0.0; 2];
        process_communication(
            &[0.8, -0.8],
            &[0.2, -0.3],
            unity(),
            policy(voice, true, TxState::Ready),
            &mut output,
        )
        .unwrap();
        assert_eq!(output, [0.2, -0.3]);
    }
}

#[test]
fn tx_muted_or_faulted_silences_voice_and_owned_clips() {
    for tx in [TxState::Muted, TxState::Faulted] {
        let mut output = [0.9; 2];
        process_communication(
            &[0.8; 2],
            &[0.8; 2],
            unity(),
            policy(VoiceState::Enabled, true, tx),
            &mut output,
        )
        .unwrap();
        assert_eq!(output, [0.0; 2]);
    }
}

#[test]
fn sum_is_finite_and_clamped_to_normalized_output() {
    let mut output = [0.0; 4];
    process_communication(
        &[1.0, -1.0, 0.9, -0.9],
        &[1.0, -1.0, 0.9, -0.9],
        unity(),
        policy(VoiceState::Enabled, true, TxState::Ready),
        &mut output,
    )
    .unwrap();
    assert_eq!(output, [1.0, -1.0, 1.0, -1.0]);
    assert!(output.iter().all(|s| s.is_finite()));
}

#[test]
fn invalid_gains_fail_closed_even_when_tx_is_muted() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.01, 1.01] {
        for gains in [
            CommGains {
                voice: bad,
                clips: 0.5,
            },
            CommGains {
                voice: 0.5,
                clips: bad,
            },
        ] {
            let mut output = [0.9; 2];
            assert_eq!(
                process_communication(
                    &[0.2; 2],
                    &[0.3; 2],
                    gains,
                    policy(VoiceState::Enabled, true, TxState::Muted),
                    &mut output
                ),
                Err(AudioError::InvalidGain)
            );
            assert_eq!(output, [0.0; 2]);
        }
    }
}

#[test]
fn invalid_samples_fail_closed_even_on_a_gated_source() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.01, 1.01] {
        for (voice, clips) in [([bad, 0.2], [0.3, 0.4]), ([0.1, 0.2], [bad, 0.4])] {
            let mut output = [0.9; 2];
            assert_eq!(
                process_communication(
                    &voice,
                    &clips,
                    unity(),
                    policy(VoiceState::Disabled, false, TxState::Ready),
                    &mut output
                ),
                Err(AudioError::InvalidSample)
            );
            assert_eq!(output, [0.0; 2]);
        }
    }
}

#[test]
fn mismatched_communication_lengths_clear_the_entire_output() {
    for (voice, clips) in [(vec![0.1], vec![0.2; 2]), (vec![0.1; 2], vec![0.2])] {
        let mut output = [0.9; 2];
        assert_eq!(
            process_communication(
                &voice,
                &clips,
                unity(),
                policy(VoiceState::Enabled, true, TxState::Ready),
                &mut output
            ),
            Err(AudioError::LengthMismatch)
        );
        assert_eq!(output, [0.0; 2]);
    }
}
