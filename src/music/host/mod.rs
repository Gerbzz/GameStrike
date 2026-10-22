//! Optional Windows local-music host using Symphonia and CPAL.
//! Factories are idle until explicit preparation and Play requests. The caller
//! supplies a selected output handle; this module never discovers microphone
//! inputs, selects a default output or changes a system device.
//! CPAL has no strict physical-drain acknowledgement, so automatic natural-end
//! advance remains unavailable. The desktop keeps this backend unattached until
//! completion and device lifecycle integration are validated.

pub mod buffer;
pub mod decoder;
pub mod output;

use super::{service::PlaybackService, streaming::StreamingBackend};
use decoder::SymphoniaDecoderFactory;
use output::CpalOutputFactory;
use std::sync::Arc;

/// Construct an idle backend for a caller-selected output. No I/O or audio starts
/// here; no source path is opened by construction.
pub fn backend_for_selected_output(
    device: cpal::Device,
) -> StreamingBackend<SymphoniaDecoderFactory, CpalOutputFactory> {
    StreamingBackend::new(
        SymphoniaDecoderFactory::new(),
        CpalOutputFactory::selected(device),
    )
}

/// Create only the transport actor. It has an empty queue and no Play authority
/// until the caller explicitly supplies metadata and issues a Play request.
/// Device creation, decoding and CPAL stream destruction stay off the UI thread.
pub fn service_for_selected_output(
    device: cpal::Device,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> PlaybackService {
    PlaybackService::start_with_waker(
        move || Ok(Box::new(backend_for_selected_output(device))),
        wake,
    )
}
