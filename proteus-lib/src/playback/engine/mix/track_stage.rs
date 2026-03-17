//! Logical-track mixing helpers.

use crate::dsp::guardrails::{sanitize_finite_clamped, sanitize_finite_min};

/// Apply per-track gain/pan in-place to interleaved samples.
///
/// Stereo-only panning is supported for now.
pub(crate) fn apply_track_gain_pan(samples: &mut [f32], level: f32, pan: f32, channels: usize) {
    let level = sanitize_finite_min(level, 1.0, 0.0);
    if channels <= 1 {
        for sample in samples.iter_mut() {
            *sample *= level;
        }
        return;
    }

    let pan = sanitize_finite_clamped(pan, 0.0, -1.0, 1.0);

    let left = if pan > 0.0 { 1.0 - pan } else { 1.0 };
    let right = if pan < 0.0 { 1.0 + pan } else { 1.0 };

    for (sample_index, sample) in samples.iter_mut().enumerate() {
        let lane_gain = match sample_index % channels {
            0 => left,
            1 => right,
            _ => 1.0,
        };
        *sample *= level * lane_gain;
    }
}

/// Combine logical-track buffers into one output buffer with equal weighting.
pub(crate) fn combine_tracks_equal_weight(track_buffers: &[Vec<f32>]) -> Vec<f32> {
    let Some(first) = track_buffers.first() else {
        return Vec::new();
    };
    let len = first.len();
    if len == 0 {
        return Vec::new();
    }

    let weight = 1.0_f32 / track_buffers.len() as f32;
    let mut out = vec![0.0_f32; len];
    for buffer in track_buffers {
        for (sample_index, sample) in buffer.iter().copied().enumerate().take(len) {
            out[sample_index] += sample * weight;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Verifies full-left pan mutes the right lane for stereo samples.
    fn apply_track_gain_pan_handles_stereo() {
        let mut samples = vec![1.0_f32, 1.0, 0.5, 0.5];
        apply_track_gain_pan(&mut samples, 1.0, -1.0, 2);
        assert_eq!(samples, vec![1.0_f32, 0.0, 0.5, 0.0]);
    }

    #[test]
    /// Verifies equal-weight mixing averages corresponding sample lanes.
    fn combine_tracks_equal_weight_averages_tracks() {
        let out = combine_tracks_equal_weight(&[
            vec![1.0_f32, 0.0, 1.0, 0.0],
            vec![0.0_f32, 1.0, 0.0, 1.0],
        ]);
        assert_eq!(out, vec![0.5_f32, 0.5, 0.5, 0.5]);
    }

    #[test]
    fn apply_track_gain_pan_clamps_invalid_inputs() {
        let mut samples = vec![1.0_f32, 1.0, 1.0, 1.0];
        apply_track_gain_pan(&mut samples, f32::NAN, 2.0, 2);
        assert_eq!(samples, vec![0.0_f32, 1.0, 0.0, 1.0]);
    }
}
