//! Time-domain level metering helpers.

use crate::dsp::guardrails::sanitize_channels;

use super::LevelSnapshot;

/// Measure per-channel peak and RMS levels from an interleaved PCM buffer.
pub fn measure_peak_rms(samples: &[f32], channels: usize) -> LevelSnapshot {
    let mut snapshot = LevelSnapshot::default();
    measure_peak_rms_into(samples, channels, &mut snapshot);
    snapshot
}

/// Measure per-channel peak and RMS levels into an existing snapshot buffer.
pub(crate) fn measure_peak_rms_into(
    samples: &[f32],
    channels: usize,
    snapshot: &mut LevelSnapshot,
) {
    let channels = sanitize_channels(channels);
    resize_level_snapshot(snapshot, channels);
    snapshot.peak.fill(0.0);
    snapshot.rms.fill(0.0);

    if samples.is_empty() {
        return;
    }

    let mut frames_per_channel = vec![0_usize; channels];
    for (index, sample) in samples.iter().copied().enumerate() {
        let channel = index % channels;
        let magnitude = sample.abs();
        if magnitude > snapshot.peak[channel] {
            snapshot.peak[channel] = magnitude;
        }
        snapshot.rms[channel] += sample * sample;
        frames_per_channel[channel] += 1;
    }

    for (channel, sum_squares) in snapshot.rms.iter_mut().enumerate() {
        let frames = frames_per_channel[channel].max(1) as f32;
        *sum_squares = (*sum_squares / frames).sqrt();
    }
}

pub(crate) fn resize_level_snapshot(snapshot: &mut LevelSnapshot, channels: usize) {
    let channels = sanitize_channels(channels);
    if snapshot.peak.len() != channels {
        snapshot.peak.resize(channels, 0.0);
    }
    if snapshot.rms.len() != channels {
        snapshot.rms.resize(channels, 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::measure_peak_rms;

    #[test]
    fn measure_peak_rms_tracks_known_stereo_signal() {
        let samples = vec![1.0_f32, 0.0, -1.0, 0.5, 0.5, -0.5, -0.5, 0.0];

        let snapshot = measure_peak_rms(&samples, 2);

        assert_eq!(snapshot.peak, vec![1.0, 0.5]);
        assert!((snapshot.rms[0] - 0.7905694).abs() < 1.0e-6);
        assert!((snapshot.rms[1] - 0.35355338).abs() < 1.0e-6);
    }

    #[test]
    fn measure_peak_rms_returns_zeroes_for_empty_input() {
        let snapshot = measure_peak_rms(&[], 2);
        assert_eq!(snapshot.peak, vec![0.0, 0.0]);
        assert_eq!(snapshot.rms, vec![0.0, 0.0]);
    }
}
