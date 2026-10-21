#![forbid(unsafe_code)]
mod common;

use common::*;
use gamestrike_audio_core::*;

#[test]
fn session_starts_with_voice_disabled_endpoint_unavailable_and_zero_mic_opens() {
    let (input, trace) = mic();
    let session = Session::new(input, 16);
    assert_eq!(session.snapshot(), SessionSnapshot {
        voice: VoiceState::Disabled, tx: TxState::Ready,
        endpoint: EndpointAvailability::Unavailable,
        queued_voice_frames: 0, queued_clip_frames: 0, preset: VoicePreset::Bypass,
    });
    assert!(trace.borrow().events.is_empty());
}

#[test]
fn explicit_voice_enable_cannot_open_input_without_a_verified_endpoint() {
    let (input, trace) = mic();
    let mut session = Session::new(input, 16);
    assert_eq!(session.enable_voice(&selected_mic()), Err(AudioError::EndpointUnavailable));
    assert!(trace.borrow().events.is_empty());
    assert_eq!(session.snapshot().voice, VoiceState::Disabled);
}

#[test]
fn endpoint_connection_alone_never_opens_a_microphone() {
    let (mut session, trace, _bridge, _) = ready_session(16);
    assert!(trace.borrow().events.is_empty());
    assert_eq!(session.snapshot().voice, VoiceState::Disabled);
    assert_eq!(session.snapshot().endpoint, EndpointAvailability::Verified);
    session.enable_voice(&selected_mic()).unwrap();
    assert_eq!(trace.borrow().events, vec![MicEvent::Open(selected_mic())]);
    assert_eq!(session.snapshot().voice, VoiceState::Enabled);
}

#[test]
fn clips_transmit_without_opening_or_enabling_a_microphone() {
    let (mut session, trace, _bridge, _) = ready_session(16);
    session.enqueue_clip(&[0.2, -0.3, 0.4]).unwrap();
    let mut output = [0.9; 3];
    session.render(unity(), false, &mut output).unwrap();
    assert_eq!(output, [0.2, -0.3, 0.4]);
    assert!(trace.borrow().events.is_empty());
    assert_eq!(session.snapshot().voice, VoiceState::Disabled);
}

#[test]
fn disabled_voice_cannot_accumulate_microphone_samples() {
    let (mut session, trace, _bridge, _) = ready_session(16);
    assert_eq!(session.enqueue_voice(&[0.5; 4]), Err(AudioError::VoiceDisabled));
    assert_eq!(session.snapshot().queued_voice_frames, 0);
    assert!(trace.borrow().events.is_empty());
}

#[test]
fn voice_mute_and_disable_release_input_and_flush_both_pending_queues() {
    for mute in [false, true] {
        let (mut session, trace, _bridge, _) = ready_session(16);
        session.enable_voice(&selected_mic()).unwrap();
        session.set_preset(VoicePreset::Robot).unwrap();
        session.enqueue_voice(&[0.6; 4]).unwrap();
        session.enqueue_clip(&[0.2; 4]).unwrap();
        if mute { session.mute_voice().unwrap(); } else { session.disable_voice().unwrap(); }
        let snapshot = session.snapshot();
        assert_eq!(snapshot.voice, if mute { VoiceState::Muted } else { VoiceState::Disabled });
        assert_eq!((snapshot.queued_voice_frames, snapshot.queued_clip_frames), (0, 0));
        assert_eq!(trace.borrow().events, vec![MicEvent::Open(selected_mic()), MicEvent::Release]);
        let mut output = [0.9; 4];
        session.render(unity(), true, &mut output).unwrap();
        assert_eq!(output, [0.0; 4], "pending data must not survive a mute/disable");
        session.enqueue_clip(&[0.25; 4]).unwrap();
        session.render(unity(), false, &mut output).unwrap();
        assert_eq!(output, [0.25; 4], "new owned clips still work after voice mute/disable");
    }
}

#[test]
fn voice_release_clears_effect_history_before_explicit_reenable() {
    for mute in [false, true] {
        let (mut session, _, _bridge, _) = ready_session(1_024);
        session.enable_voice(&selected_mic()).unwrap();
        session.set_preset(VoicePreset::Robot).unwrap();
        session.enqueue_voice(&[0.6; 777]).unwrap();
        session.render(unity(), true, &mut [0.0; 777]).unwrap();
        if mute { session.mute_voice().unwrap(); } else { session.disable_voice().unwrap(); }
        session.enable_voice(&selected_mic()).unwrap();
        session.set_preset(VoicePreset::Robot).unwrap();
        session.enqueue_voice(&[0.6; 400]).unwrap();
        let mut output = [0.0; 400];
        session.render(unity(), true, &mut output).unwrap();
        let mut fresh = [0.0; 400];
        VoiceProcessor::new().process(&[0.6; 400], VoicePreset::Robot, SAMPLE_RATE_HZ, &mut fresh).unwrap();
        assert_close(&output, &fresh, 0.000_01);
    }
}

#[test]
fn tx_mute_releases_input_flushes_everything_and_resume_does_not_reopen_mic() {
    let (mut session, trace, _bridge, _) = ready_session(16);
    session.enable_voice(&selected_mic()).unwrap();
    session.enqueue_voice(&[0.6; 4]).unwrap();
    session.enqueue_clip(&[0.2; 4]).unwrap();
    session.mute_tx().unwrap();
    assert_eq!(session.snapshot().tx, TxState::Muted);
    assert_eq!(session.snapshot().voice, VoiceState::Disabled);
    assert_eq!(trace.borrow().events, vec![MicEvent::Open(selected_mic()), MicEvent::Release]);
    let mut output = [0.9; 4];
    session.render(unity(), true, &mut output).unwrap();
    assert_eq!(output, [0.0; 4]);
    session.resume_tx().unwrap();
    session.render(unity(), true, &mut output).unwrap();
    assert_eq!(output, [0.0; 4]);
    assert_eq!(trace.borrow().events.len(), 2, "resume must require a new explicit voice enable");
}

#[test]
fn fault_releases_input_flushes_queues_and_never_auto_recovers() {
    let (mut session, trace, mut bridge, _) = ready_session(16);
    session.enable_voice(&selected_mic()).unwrap();
    session.enqueue_voice(&[0.6; 4]).unwrap();
    session.enqueue_clip(&[0.2; 4]).unwrap();
    session.fault().unwrap();
    assert_eq!(session.snapshot().tx, TxState::Faulted);
    assert_eq!((session.snapshot().queued_voice_frames, session.snapshot().queued_clip_frames), (0, 0));
    assert_eq!(trace.borrow().events, vec![MicEvent::Open(selected_mic()), MicEvent::Release]);
    let new_endpoint = bridge.connect(&pair()).unwrap();
    assert_eq!(session.set_endpoint(Some(new_endpoint)), Err(AudioError::Faulted));
    assert_eq!(session.resume_tx(), Err(AudioError::Faulted));
    assert_eq!(session.enable_voice(&selected_mic()), Err(AudioError::Faulted));
    assert_eq!(session.set_preset(VoicePreset::Robot), Err(AudioError::Faulted));
    assert_eq!(session.enqueue_clip(&[0.2; 4]), Err(AudioError::Faulted));
    let mut output = [0.9; 4];
    session.render(unity(), true, &mut output).unwrap();
    assert_eq!(output, [0.0; 4]);
    assert_eq!(trace.borrow().events.len(), 2);
}

#[test]
fn endpoint_loss_releases_input_and_flushes_pending_data() {
    let (mut session, trace, _bridge, _) = ready_session(16);
    session.enable_voice(&selected_mic()).unwrap();
    session.enqueue_voice(&[0.6; 4]).unwrap();
    session.enqueue_clip(&[0.2; 4]).unwrap();
    session.set_endpoint(None).unwrap();
    assert_eq!(session.snapshot().endpoint, EndpointAvailability::Unavailable);
    assert_eq!(session.snapshot().tx, TxState::Faulted);
    assert_eq!(trace.borrow().events, vec![MicEvent::Open(selected_mic()), MicEvent::Release]);
    let mut output = [0.9; 4];
    session.render(unity(), true, &mut output).unwrap();
    assert_eq!(output, [0.0; 4]);
}

#[test]
fn revoked_bridge_proof_cannot_authorize_a_later_mic_open() {
    let (mut session, trace, mut bridge, _) = ready_session(16);
    bridge.disconnect().unwrap();
    assert_eq!(session.enable_voice(&selected_mic()), Err(AudioError::EndpointUnavailable));
    assert!(trace.borrow().events.is_empty());
}

#[test]
fn revoked_proof_while_voice_is_enabled_faults_and_flushes_on_next_render() {
    let (mut session, trace, mut bridge) = primed_session();
    bridge.disconnect().unwrap();
    let mut output = [0.9; 4];
    assert_eq!(session.render(unity(), true, &mut output), Err(AudioError::EndpointUnavailable));
    assert_eq!(output, [0.0; 4]);
    assert_eq!(session.snapshot().endpoint, EndpointAvailability::Unavailable);
    assert_terminal_fault(&mut session, &trace);
    let new_proof = bridge.connect(&pair()).unwrap();
    assert_eq!(session.set_endpoint(Some(new_proof)), Err(AudioError::Faulted));
    assert_terminal_fault(&mut session, &trace);
}

#[test]
fn closed_ptt_consumes_live_voice_and_never_replays_it_after_reopening() {
    let (mut session, trace, _bridge, _) = ready_session(16);
    session.enable_voice(&selected_mic()).unwrap();
    session.enqueue_voice(&[0.9, -0.9, 0.7, -0.7]).unwrap();
    session.enqueue_clip(&[0.1, 0.2, -0.1, -0.2]).unwrap();
    let mut output = [0.9; 4];
    session.render(unity(), false, &mut output).unwrap();
    assert_eq!(output, [0.1, 0.2, -0.1, -0.2]);
    assert_eq!((session.snapshot().queued_voice_frames, session.snapshot().queued_clip_frames), (0, 0));
    assert_eq!(session.snapshot().voice, VoiceState::Enabled);
    assert_eq!(trace.borrow().events, vec![MicEvent::Open(selected_mic())]);
    session.enqueue_clip(&[0.05; 4]).unwrap();
    session.render(unity(), true, &mut output).unwrap();
    assert_eq!(output, [0.05; 4], "voice captured while PTT was closed must never replay");
    assert_eq!(trace.borrow().events, vec![MicEvent::Open(selected_mic())]);
}

#[test]
fn closed_ptt_advances_live_robot_phase_and_transition_without_releasing_input() {
    let (mut session, trace, _bridge, _) = ready_session(1_024);
    session.enable_voice(&selected_mic()).unwrap();
    session.set_preset(VoicePreset::Robot).unwrap();
    session.enqueue_voice(&[0.6; 777]).unwrap();
    session.enqueue_clip(&[0.1; 777]).unwrap();
    let mut closed_output = [0.9; 777];
    session.render(unity(), false, &mut closed_output).unwrap();
    assert_eq!(closed_output, [0.1; 777]);
    assert_eq!(session.snapshot().queued_voice_frames, 0);
    session.enqueue_voice(&[0.6; 400]).unwrap();
    let mut reopened = [0.0; 400];
    session.render(unity(), true, &mut reopened).unwrap();
    let mut reference = VoiceProcessor::new();
    reference.process(&[0.6; 777], VoicePreset::Robot, SAMPLE_RATE_HZ, &mut [0.0; 777]).unwrap();
    let mut expected = [0.0; 400];
    reference.process(&[0.6; 400], VoicePreset::Robot, SAMPLE_RATE_HZ, &mut expected).unwrap();
    assert_close(&reopened, &expected, 0.000_01);
    assert_eq!(trace.borrow().events, vec![MicEvent::Open(selected_mic())]);
}

fn primed_session() -> (
    Session<FakeMic>, std::rc::Rc<std::cell::RefCell<MicTrace>>, EndpointBridge<FakeBackend>,
) {
    let (mut session, trace, bridge, _) = ready_session(1_024);
    session.enable_voice(&selected_mic()).unwrap();
    session.set_preset(VoicePreset::Robot).unwrap();
    session.enqueue_voice(&[0.6; 777]).unwrap();
    session.render(unity(), true, &mut [0.0; 777]).unwrap();
    session.enqueue_voice(&[0.7; 4]).unwrap();
    session.enqueue_clip(&[0.2; 4]).unwrap();
    assert_eq!((session.snapshot().queued_voice_frames, session.snapshot().queued_clip_frames), (4, 4));
    (session, trace, bridge)
}

fn assert_terminal_fault(session: &mut Session<FakeMic>,
    trace: &std::rc::Rc<std::cell::RefCell<MicTrace>>) {
    let snapshot = session.snapshot();
    assert_eq!(snapshot.tx, TxState::Faulted);
    assert_eq!(snapshot.voice, VoiceState::Disabled);
    assert_eq!((snapshot.queued_voice_frames, snapshot.queued_clip_frames), (0, 0));
    assert_eq!(snapshot.preset, VoicePreset::Bypass, "fatal cleanup deactivates effects");
    assert_eq!(trace.borrow().events, vec![MicEvent::Open(selected_mic()), MicEvent::Release]);
    for preset in [VoicePreset::Bypass, VoicePreset::Robot] {
        assert_eq!(session.set_preset(preset), Err(AudioError::Faulted));
    }
    assert_eq!(session.resume_tx(), Err(AudioError::Faulted));
    assert_eq!(session.enable_voice(&selected_mic()), Err(AudioError::Faulted));
    assert_eq!(session.enqueue_voice(&[0.1; 4]), Err(AudioError::Faulted));
    assert_eq!(session.enqueue_clip(&[0.2; 4]), Err(AudioError::Faulted));
    let mut output = [0.9; 4];
    session.render(unity(), true, &mut output).unwrap();
    assert_eq!(output, [0.0; 4]);
    assert_eq!(trace.borrow().events.len(), 2, "no operation may reopen a faulted input");
    assert_eq!(session.snapshot(), snapshot, "failed revival cannot change fault state");
}

fn assert_invalid_queued_sample_faults(voice: bool) {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.01, -1.01] {
        let (mut session, trace, _bridge) = primed_session();
        let invalid = [0.25, bad, -0.25];
        let result = if voice { session.enqueue_voice(&invalid) } else { session.enqueue_clip(&invalid) };
        assert_eq!(result, Err(AudioError::InvalidSample));
        assert_terminal_fault(&mut session, &trace);
    }
}

#[test]
fn invalid_queued_voice_faults_releases_input_and_flushes_voice_clips_and_effects() {
    assert_invalid_queued_sample_faults(true);
}

#[test]
fn invalid_queued_clips_fault_release_input_and_flush_voice_clips_and_effects() {
    assert_invalid_queued_sample_faults(false);
}

#[test]
fn invalid_render_gains_clear_output_and_latch_complete_session_cleanup() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.01, 1.01] {
        for gains in [CommGains { voice: bad, clips: 0.5 }, CommGains { voice: 0.5, clips: bad }] {
            let (mut session, trace, _bridge) = primed_session();
            let mut output = [0.9; 4];
            assert_eq!(session.render(gains, true, &mut output), Err(AudioError::InvalidGain));
            assert_eq!(output, [0.0; 4]);
            assert_terminal_fault(&mut session, &trace);
        }
    }
}

#[test]
fn failed_explicit_input_open_faults_without_trying_a_default_or_substitute() {
    let (mut session, trace, _bridge, _) = ready_session(16);
    trace.borrow_mut().fail_open = true;
    assert_eq!(session.enable_voice(&selected_mic()), Err(AudioError::InputFailure));
    assert_eq!(session.snapshot().tx, TxState::Faulted);
    let opens: Vec<_> = trace.borrow().events.iter().filter_map(|event| match event {
        MicEvent::Open(id) => Some(id.clone()), MicEvent::Release => None,
    }).collect();
    assert_eq!(opens, vec![selected_mic()]);
    assert_eq!(session.resume_tx(), Err(AudioError::Faulted));
}

#[test]
fn session_queues_are_bounded_without_partial_append() {
    let (mut session, _, _bridge, _) = ready_session(4);
    session.enable_voice(&selected_mic()).unwrap();
    session.enqueue_voice(&[0.1; 3]).unwrap();
    session.enqueue_clip(&[0.2; 3]).unwrap();
    assert_eq!(session.enqueue_voice(&[0.9; 2]), Err(AudioError::QueueFull));
    assert_eq!(session.enqueue_clip(&[0.9; 2]), Err(AudioError::QueueFull));
    assert_eq!((session.snapshot().queued_voice_frames, session.snapshot().queued_clip_frames), (3, 3));
    let mut output = [0.0; 4];
    session.render(unity(), true, &mut output).unwrap();
    assert_close(&output, &[0.3, 0.3, 0.3, 0.0], 0.000_01);
}

#[test]
fn dropping_an_enabled_session_releases_the_explicit_input() {
    let (mut session, trace, _bridge, _) = ready_session(16);
    session.enable_voice(&selected_mic()).unwrap();
    drop(session);
    assert_eq!(trace.borrow().events, vec![MicEvent::Open(selected_mic()), MicEvent::Release]);
}
