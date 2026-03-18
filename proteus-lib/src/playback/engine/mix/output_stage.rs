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
///
/// When `output_slice_samples` is `Some(n)`, the buffer is sliced into chunks
/// of at most `n` samples (frame-aligned) and each slice is sent individually.
/// The bounded `sync_channel(1)` between mix and worker threads naturally
/// gates each send, providing per-slice backpressure. When `None`, the entire
/// buffer is sent as a single chunk (the default behavior).
pub(super) fn send_samples(
    sender: &mpsc::SyncSender<(SamplesBuffer, f64)>,
    input_channels: u16,
    sample_rate: u32,
    samples: &[f32],
    output_slice_samples: Option<usize>,
) -> SendStatus {
    if samples.is_empty() {
        return SendStatus::Empty;
    }

    let max_chunk = output_slice_samples
        .unwrap_or(samples.len())
        .max(input_channels as usize);

    for chunk in samples.chunks(max_chunk) {
        let length_in_seconds = chunk.len() as f64 / sample_rate as f64 / input_channels as f64;
        let samples_buffer = SamplesBuffer::new(input_channels, sample_rate, chunk.to_vec());

        if let Err(e) = sender.send((samples_buffer, length_in_seconds)) {
            log::error!("failed to send samples: {}", e);
            return SendStatus::Disconnected;
        }
    }
    SendStatus::Sent
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_samples_returns_empty_for_empty_buffers() {
        let (tx, _rx) = mpsc::sync_channel(1);
        let status = send_samples(&tx, 2, 48_000, &[], None);
        assert!(matches!(status, SendStatus::Empty));
    }

    #[test]
    fn send_samples_returns_disconnected_when_receiver_is_gone() {
        let (tx, rx) = mpsc::sync_channel(1);
        drop(rx);
        let status = send_samples(&tx, 2, 48_000, &[0.1, -0.1], None);
        assert!(matches!(status, SendStatus::Disconnected));
    }

    #[test]
    fn send_samples_sliced_produces_multiple_chunks() {
        let (tx, rx) = mpsc::sync_channel(16);
        // 8 samples, stereo, slice into groups of 4 (2 frames each)
        let samples = [0.1, -0.1, 0.2, -0.2, 0.3, -0.3, 0.4, -0.4];
        let status = send_samples(&tx, 2, 48_000, &samples, Some(4));
        assert!(matches!(status, SendStatus::Sent));

        let (_chunk1, dur1) = rx.recv().unwrap();
        let (_chunk2, dur2) = rx.recv().unwrap();
        assert!(rx.try_recv().is_err());

        // Each slice should carry 4 samples = 2 frames at 48 kHz
        let expected_dur = 4.0 / 48_000.0 / 2.0;
        assert!((dur1 - expected_dur).abs() < 1e-9);
        assert!((dur2 - expected_dur).abs() < 1e-9);
    }

    #[test]
    fn send_samples_none_slice_sends_single_chunk() {
        let (tx, rx) = mpsc::sync_channel(16);
        let samples = [0.1, -0.1, 0.2, -0.2];
        let status = send_samples(&tx, 2, 48_000, &samples, None);
        assert!(matches!(status, SendStatus::Sent));

        let (_chunk, _dur) = rx.recv().unwrap();
        assert!(rx.try_recv().is_err());
    }
}
