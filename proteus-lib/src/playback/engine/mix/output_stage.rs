//! Output-stage DSP helpers for the mix runtime.

use rodio::buffer::SamplesBuffer;
use std::sync::mpsc;

/// Send produced samples over the mix thread output channel.
pub(super) enum SendStatus {
    Sent,
    Empty,
    Disconnected,
}

/// Send produced samples over the mix thread output channel.
pub(super) fn send_samples(
    sender: &mpsc::SyncSender<(SamplesBuffer, f64)>,
    input_channels: u16,
    sample_rate: u32,
    samples: Vec<f32>,
) -> SendStatus {
    if samples.is_empty() {
        return SendStatus::Empty;
    }

    let length_in_seconds = samples.len() as f64 / sample_rate as f64 / input_channels as f64;
    let samples_buffer = SamplesBuffer::new(input_channels, sample_rate, samples);

    if let Err(e) = sender.send((samples_buffer, length_in_seconds)) {
        log::error!("Failed to send samples: {}", e);
        return SendStatus::Disconnected;
    }
    // info!("Samples sent successfully of length {}", length_in_seconds);
    SendStatus::Sent
}
