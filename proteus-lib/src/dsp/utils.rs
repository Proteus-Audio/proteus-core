//! Shared DSP helper utilities.

/// Apply a linear gain ramp across interleaved audio frames.
///
/// # Arguments
///
/// * `samples` - Interleaved audio samples to scale in place.
/// * `channels` - Channel count used to interpret frame boundaries.
/// * `start` - Gain applied to the first frame.
/// * `end` - Gain applied to the final frame.
pub fn fade_interleaved_per_frame(samples: &mut [f32], channels: usize, start: f32, end: f32) {
    if channels == 0 {
        return;
    }
    let samples_len = samples.len();
    let frames = samples_len / channels;
    if frames == 0 {
        return;
    }
    if frames == 1 {
        let gain = end;
        for s in &mut samples[..channels.min(samples_len)] {
            *s *= gain;
        }
        return;
    }

    let step = (end - start) / (frames as f32 - 1.0);
    let mut gain = start;

    for frame in samples[..frames * channels].chunks_exact_mut(channels) {
        for s in frame.iter_mut() {
            *s *= gain;
        }
        gain += step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fade_interleaved_per_frame_applies_linear_ramp() {
        let mut samples = vec![1.0_f32, 1.0, 1.0, 1.0];
        fade_interleaved_per_frame(&mut samples, 2, 0.0, 1.0);
        assert_eq!(samples, vec![0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn fade_interleaved_per_frame_handles_single_frame() {
        let mut samples = vec![2.0_f32, -2.0];
        fade_interleaved_per_frame(&mut samples, 2, 0.2, 0.5);
        assert_eq!(samples, vec![1.0, -1.0]);
    }

    #[test]
    fn fade_interleaved_per_frame_ignores_zero_channels() {
        let mut samples = vec![2.0_f32, -2.0];
        fade_interleaved_per_frame(&mut samples, 0, 0.2, 0.5);
        assert_eq!(samples, vec![2.0, -2.0]);
    }
}
