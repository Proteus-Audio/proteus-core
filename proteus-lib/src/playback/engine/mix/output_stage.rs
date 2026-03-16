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
        log::error!("failed to send samples: {}", e);
        return SendStatus::Disconnected;
    }
    SendStatus::Sent
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_samples_returns_empty_for_empty_buffers() {
        let (tx, _rx) = mpsc::sync_channel(1);
        let status = send_samples(&tx, 2, 48_000, Vec::new());
        assert!(matches!(status, SendStatus::Empty));
    }

    #[test]
    fn send_samples_returns_disconnected_when_receiver_is_gone() {
        let (tx, rx) = mpsc::sync_channel(1);
        drop(rx);
        let status = send_samples(&tx, 2, 48_000, vec![0.1, -0.1]);
        assert!(matches!(status, SendStatus::Disconnected));
    }
}
