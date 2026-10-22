#![allow(dead_code)]

use gamestrike_audio_core::*;
use std::cell::RefCell;
use std::rc::Rc;

pub fn pair() -> EndpointPairId {
    EndpointPairId {
        render: "stable-render-17".into(),
        capture: "stable-capture-29".into(),
    }
}

pub fn format(encoding: SampleEncoding, channels: u16) -> NegotiatedFormat {
    NegotiatedFormat {
        sample_rate: SAMPLE_RATE_HZ,
        channels,
        encoding,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Write {
    pub format: NegotiatedFormat,
    pub samples: DeviceSamples,
}

pub struct BackendTrace {
    pub handshake: Result<BackendHandshake, AudioError>,
    pub connected: bool,
    pub requests: Vec<EndpointPairId>,
    pub writes: Vec<Write>,
    pub closes: usize,
}

#[derive(Clone)]
pub struct FakeBackend(pub Rc<RefCell<BackendTrace>>);

pub fn backend(format: NegotiatedFormat) -> (FakeBackend, Rc<RefCell<BackendTrace>>) {
    let trace = Rc::new(RefCell::new(BackendTrace {
        handshake: Ok(BackendHandshake {
            opened_pair: pair(),
            backend_session_id: "backend-open-session-1".into(),
            format,
        }),
        connected: true,
        requests: Vec::new(),
        writes: Vec::new(),
        closes: 0,
    }));
    (FakeBackend(trace.clone()), trace)
}

impl EndpointBackend for FakeBackend {
    fn handshake(&mut self, requested: &EndpointPairId) -> Result<BackendHandshake, AudioError> {
        let mut trace = self.0.borrow_mut();
        trace.requests.push(requested.clone());
        if !trace.connected {
            return Err(AudioError::BackendDisconnected);
        }
        trace.handshake.clone()
    }
    fn write(
        &mut self,
        format: NegotiatedFormat,
        samples: &DeviceSamples,
    ) -> Result<(), AudioError> {
        let mut trace = self.0.borrow_mut();
        if !trace.connected {
            return Err(AudioError::BackendDisconnected);
        }
        trace.writes.push(Write {
            format,
            samples: samples.clone(),
        });
        Ok(())
    }
    fn close(&mut self) {
        self.0.borrow_mut().closes += 1;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MicEvent {
    Open(InputDeviceId),
    Release,
}

#[derive(Default)]
pub struct MicTrace {
    pub events: Vec<MicEvent>,
    pub fail_open: bool,
}

#[derive(Clone)]
pub struct FakeMic(pub Rc<RefCell<MicTrace>>);

pub fn mic() -> (FakeMic, Rc<RefCell<MicTrace>>) {
    let trace = Rc::new(RefCell::new(MicTrace::default()));
    (FakeMic(trace.clone()), trace)
}

impl MicInput for FakeMic {
    fn open(&mut self, device: &InputDeviceId) -> Result<(), AudioError> {
        let mut trace = self.0.borrow_mut();
        trace.events.push(MicEvent::Open(device.clone()));
        if trace.fail_open {
            Err(AudioError::InputFailure)
        } else {
            Ok(())
        }
    }
    fn release(&mut self) {
        self.0.borrow_mut().events.push(MicEvent::Release);
    }
}

pub fn selected_mic() -> InputDeviceId {
    InputDeviceId("explicit-input-41".into())
}

pub fn ready_session(
    capacity: usize,
) -> (
    Session<FakeMic>,
    Rc<RefCell<MicTrace>>,
    EndpointBridge<FakeBackend>,
    Rc<RefCell<BackendTrace>>,
) {
    let (backend, trace) = backend(format(SampleEncoding::F32, 1));
    let mut bridge = EndpointBridge::new(backend, capacity);
    let endpoint = bridge
        .connect(&pair())
        .expect("synthetic endpoint should verify");
    let (mic, mic_trace) = mic();
    let mut session = Session::new(mic, capacity);
    session
        .set_endpoint(Some(endpoint))
        .expect("verified endpoint should be accepted");
    (session, mic_trace, bridge, trace)
}

pub fn unity() -> CommGains {
    CommGains {
        voice: 1.0,
        clips: 1.0,
    }
}

pub fn assert_close(actual: &[f32], expected: &[f32], tolerance: f32) {
    assert_eq!(actual.len(), expected.len());
    for (index, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
        assert!(
            (actual - expected).abs() <= tolerance,
            "sample {index}: expected {expected}, got {actual}"
        );
    }
}
