#![forbid(unsafe_code)]
mod common;

use common::*;
use gamestrike_audio_core::*;

#[test]
fn endpoint_is_unavailable_until_actual_backend_handshake() {
    let (backend, trace) = backend(format(SampleEncoding::F32, 1));
    let mut bridge = EndpointBridge::new(backend, 16);
    assert_eq!(bridge.availability(), EndpointAvailability::Unavailable);
    assert!(trace.borrow().requests.is_empty());
    assert_eq!(bridge.enqueue(&[0.2]), Err(AudioError::EndpointUnavailable));
    let proof = bridge.connect(&pair()).unwrap();
    assert_eq!(proof.pair(), &pair());
    assert!(proof.is_available());
    assert_eq!(trace.borrow().requests, vec![pair()]);
    assert_eq!(bridge.availability(), EndpointAvailability::Verified);
}

#[test]
fn matching_display_names_cannot_replace_stable_backend_pair_ids() {
    // Friendly names are intentionally absent from the authorization API.
    for wrong_render in [true, false] {
        let (backend, trace) = backend(format(SampleEncoding::F32, 1));
        if let Ok(proof) = &mut trace.borrow_mut().handshake {
            if wrong_render { proof.opened_pair.render = "other-stable-render".into(); }
            else { proof.opened_pair.capture = "other-stable-capture".into(); }
        }
        let mut bridge = EndpointBridge::new(backend, 16);
        assert!(matches!(bridge.connect(&pair()), Err(AudioError::EndpointProofRejected)));
        assert_eq!(bridge.availability(), EndpointAvailability::Unavailable);
        assert!(trace.borrow().writes.is_empty());
    }
}

#[test]
fn absent_backend_session_proof_is_rejected_even_with_matching_pair_ids() {
    let (backend, trace) = backend(format(SampleEncoding::F32, 1));
    if let Ok(proof) = &mut trace.borrow_mut().handshake { proof.backend_session_id.clear(); }
    let mut bridge = EndpointBridge::new(backend, 16);
    assert!(matches!(bridge.connect(&pair()), Err(AudioError::EndpointProofRejected)));
    assert_eq!(bridge.availability(), EndpointAvailability::Unavailable);
}

#[test]
fn backend_failure_never_yields_a_verified_endpoint() {
    let (backend, trace) = backend(format(SampleEncoding::F32, 1));
    trace.borrow_mut().handshake = Err(AudioError::BackendDisconnected);
    let mut bridge = EndpointBridge::new(backend, 16);
    assert!(matches!(bridge.connect(&pair()), Err(AudioError::BackendDisconnected)));
    assert_eq!(bridge.availability(), EndpointAvailability::Unavailable);
}

#[test]
fn negotiated_stereo_i16_conversion_duplicates_channels_and_reaches_backend() {
    let negotiated = format(SampleEncoding::I16, 2);
    let (backend, trace) = backend(negotiated);
    let mut bridge = EndpointBridge::new(backend, 16);
    bridge.connect(&pair()).unwrap();
    bridge.enqueue(&[-1.0, -0.5, 0.0, 0.5, 1.0]).unwrap();
    let block = bridge.pump(5).unwrap();
    let expected = DeviceSamples::I16(vec![i16::MIN, i16::MIN, -16_384, -16_384, 0, 0,
        16_384, 16_384, i16::MAX, i16::MAX]);
    assert_eq!(block, BridgeBlock { samples: expected.clone(), status: BridgeStatus::Ready });
    assert_eq!(trace.borrow().writes, vec![Write { format: negotiated, samples: expected }]);
    assert_eq!(bridge.queued_frames(), 0);
}

#[test]
fn negotiated_stereo_i32_conversion_is_not_internal_mono_float() {
    let negotiated = format(SampleEncoding::I32, 2);
    let (backend, trace) = backend(negotiated);
    let mut bridge = EndpointBridge::new(backend, 16);
    bridge.connect(&pair()).unwrap();
    bridge.enqueue(&[-1.0, 0.0, 1.0]).unwrap();
    let expected = DeviceSamples::I32(vec![i32::MIN, i32::MIN, 0, 0, i32::MAX, i32::MAX]);
    assert_eq!(bridge.pump(3).unwrap().samples, expected);
    assert_eq!(trace.borrow().writes[0].format, negotiated);
    assert!(matches!(&trace.borrow().writes[0].samples, DeviceSamples::I32(_)));
}

#[test]
fn negotiated_stereo_float_output_has_two_samples_per_frame() {
    let (backend, trace) = backend(format(SampleEncoding::F32, 2));
    let mut bridge = EndpointBridge::new(backend, 16);
    bridge.connect(&pair()).unwrap();
    bridge.enqueue(&[0.25, -0.5]).unwrap();
    assert_eq!(bridge.pump(2).unwrap().samples, DeviceSamples::F32(vec![0.25, 0.25, -0.5, -0.5]));
    assert_eq!(trace.borrow().writes.len(), 1);
}

#[test]
fn unsupported_rates_channels_and_encodings_are_rejected_before_writes() {
    for bad_format in [
        NegotiatedFormat { sample_rate: 44_100, ..format(SampleEncoding::F32, 1) },
        format(SampleEncoding::F32, 0), format(SampleEncoding::F32, 6),
        format(SampleEncoding::Unsupported, 2),
    ] {
        let (backend, trace) = backend(bad_format);
        let mut bridge = EndpointBridge::new(backend, 16);
        assert!(matches!(bridge.connect(&pair()), Err(AudioError::UnsupportedFormat)));
        assert_eq!(bridge.availability(), EndpointAvailability::Unavailable);
        assert!(trace.borrow().writes.is_empty());
    }
}

#[test]
fn underflow_writes_silence_and_does_not_replay_a_previous_block() {
    let (backend, trace) = backend(format(SampleEncoding::I16, 2));
    let mut bridge = EndpointBridge::new(backend, 16);
    bridge.connect(&pair()).unwrap();
    bridge.enqueue(&[0.5, -0.5]).unwrap();
    assert_eq!(bridge.pump(2).unwrap().status, BridgeStatus::Ready);
    let silent = bridge.pump(2).unwrap();
    assert_eq!(silent, BridgeBlock {
        samples: DeviceSamples::I16(vec![0; 4]), status: BridgeStatus::SilentUnderflow,
    });
    assert_eq!(trace.borrow().writes[1].samples, DeviceSamples::I16(vec![0; 4]));
}

#[test]
fn partial_underflow_discards_incomplete_block_instead_of_replaying_it_later() {
    let (backend, _) = backend(format(SampleEncoding::F32, 1));
    let mut bridge = EndpointBridge::new(backend, 16);
    bridge.connect(&pair()).unwrap();
    bridge.enqueue(&[0.6]).unwrap();
    assert_eq!(bridge.pump(2).unwrap(), BridgeBlock {
        samples: DeviceSamples::F32(vec![0.0; 2]), status: BridgeStatus::SilentUnderflow,
    });
    assert_eq!(bridge.queued_frames(), 0);
    bridge.enqueue(&[0.2, 0.3]).unwrap();
    assert_eq!(bridge.pump(2).unwrap().samples, DeviceSamples::F32(vec![0.2, 0.3]));
}

#[test]
fn disconnect_revokes_proof_returns_silence_and_flushes_stale_audio() {
    let (backend, trace) = backend(format(SampleEncoding::F32, 1));
    let mut bridge = EndpointBridge::new(backend, 16);
    let proof = bridge.connect(&pair()).unwrap();
    bridge.enqueue(&[0.6; 4]).unwrap();
    trace.borrow_mut().connected = false;
    assert_eq!(bridge.pump(2).unwrap(), BridgeBlock {
        samples: DeviceSamples::F32(vec![0.0; 2]), status: BridgeStatus::Disconnected,
    });
    assert!(!proof.is_available());
    assert_eq!(bridge.availability(), EndpointAvailability::Unavailable);
    assert_eq!(bridge.queued_frames(), 0);
    assert!(trace.borrow().writes.is_empty());
    trace.borrow_mut().connected = true;
    bridge.connect(&pair()).unwrap();
    assert_eq!(bridge.pump(2).unwrap().samples, DeviceSamples::F32(vec![0.0; 2]));
    assert!(!proof.is_available(), "a new handshake must not revive the old capability");
}

#[test]
fn explicit_disconnect_closes_backend_and_revokes_existing_proof() {
    let (backend, trace) = backend(format(SampleEncoding::F32, 1));
    let mut bridge = EndpointBridge::new(backend, 16);
    let proof = bridge.connect(&pair()).unwrap();
    bridge.enqueue(&[0.5; 3]).unwrap();
    bridge.disconnect().unwrap();
    assert!(!proof.is_available());
    assert_eq!(bridge.queued_frames(), 0);
    assert_eq!(trace.borrow().closes, 1);
    assert_eq!(bridge.availability(), EndpointAvailability::Unavailable);
}

#[test]
fn bridge_queue_is_bounded_and_rejects_partial_append() {
    let (backend, _) = backend(format(SampleEncoding::F32, 1));
    let mut bridge = EndpointBridge::new(backend, 4);
    bridge.connect(&pair()).unwrap();
    bridge.enqueue(&[0.1; 3]).unwrap();
    assert_eq!(bridge.enqueue(&[0.9; 2]), Err(AudioError::QueueFull));
    assert_eq!(bridge.queued_frames(), 3);
    assert_eq!(bridge.pump(3).unwrap().samples, DeviceSamples::F32(vec![0.1; 3]));
}

#[test]
fn invalid_queued_samples_flush_previously_accepted_endpoint_audio() {
    for bad in [f32::NAN, f32::INFINITY, -1.01, 1.01] {
        let (backend, _) = backend(format(SampleEncoding::F32, 1));
        let mut bridge = EndpointBridge::new(backend, 4);
        bridge.connect(&pair()).unwrap();
        bridge.enqueue(&[0.5; 2]).unwrap();
        assert_eq!(bridge.enqueue(&[bad]), Err(AudioError::InvalidSample));
        assert_eq!(bridge.queued_frames(), 0);
        assert_eq!(bridge.pump(2).unwrap().samples, DeviceSamples::F32(vec![0.0; 2]));
    }
}
