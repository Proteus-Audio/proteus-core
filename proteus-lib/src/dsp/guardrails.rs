//! Shared DSP guardrail helpers for sanitizing numeric parameters.
//!
//! These functions centralise the NaN/infinity guards and range clamping that
//! recur across effect implementations and mix-path code. Using a single set of
//! helpers keeps behaviour consistent and prevents partial fixes when edge-case
//! handling changes.

/// Return `value` when finite, otherwise `fallback`.
///
/// # Arguments
///
/// * `value` - The value to check.
/// * `fallback` - Returned when `value` is NaN or infinite.
pub fn sanitize_finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

/// Return `value` clamped to `[min, max]` when finite, otherwise `fallback`.
///
/// # Arguments
///
/// * `value` - The value to check and clamp.
/// * `fallback` - Returned when `value` is NaN or infinite.
/// * `min` - Lower bound of the clamped range.
/// * `max` - Upper bound of the clamped range.
pub fn sanitize_finite_clamped(value: f32, fallback: f32, min: f32, max: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

/// Return `value` with a floor of `min` when finite, otherwise `fallback`.
///
/// # Arguments
///
/// * `value` - The value to check.
/// * `fallback` - Returned when `value` is NaN or infinite.
/// * `min` - Lower bound enforced on the value.
pub fn sanitize_finite_min(value: f32, fallback: f32, min: f32) -> f32 {
    if value.is_finite() {
        value.max(min)
    } else {
        fallback
    }
}

/// Return `value` with a ceiling of `max` when finite, otherwise `fallback`.
///
/// # Arguments
///
/// * `value` - The value to check.
/// * `fallback` - Returned when `value` is NaN or infinite.
/// * `max` - Upper bound enforced on the value.
pub fn sanitize_finite_max(value: f32, fallback: f32, max: f32) -> f32 {
    if value.is_finite() {
        value.min(max)
    } else {
        fallback
    }
}

/// Ensure a channel count is at least 1.
///
/// # Arguments
///
/// * `channels` - Raw channel count.
pub fn sanitize_channels(channels: usize) -> usize {
    channels.max(1)
}

/// Ensure a sample rate is at least 1.
///
/// # Arguments
///
/// * `sample_rate` - Raw sample rate in Hz.
pub fn sanitize_sample_rate(sample_rate: u32) -> u32 {
    sample_rate.max(1)
}

/// Clamp a frequency to `[1, nyquist - 1]` based on the sample rate.
///
/// # Arguments
///
/// * `freq` - Frequency in Hz.
/// * `sample_rate` - Sample rate in Hz used to derive the Nyquist limit.
pub fn sanitize_freq(freq: u32, sample_rate: u32) -> u32 {
    let nyquist = sample_rate / 2;
    if nyquist <= 1 {
        return 1;
    }
    freq.clamp(1, nyquist.saturating_sub(1).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_finite_returns_value_when_finite() {
        assert_eq!(sanitize_finite(1.5, 0.0), 1.5);
        assert_eq!(sanitize_finite(-3.0, 1.0), -3.0);
        assert_eq!(sanitize_finite(0.0, 1.0), 0.0);
    }

    #[test]
    fn sanitize_finite_returns_fallback_for_nan_and_infinity() {
        assert_eq!(sanitize_finite(f32::NAN, 1.0), 1.0);
        assert_eq!(sanitize_finite(f32::INFINITY, 2.0), 2.0);
        assert_eq!(sanitize_finite(f32::NEG_INFINITY, -1.0), -1.0);
    }

    #[test]
    fn sanitize_finite_clamped_clamps_finite_values() {
        assert_eq!(sanitize_finite_clamped(0.5, 0.0, -1.0, 1.0), 0.5);
        assert_eq!(sanitize_finite_clamped(2.0, 0.0, -1.0, 1.0), 1.0);
        assert_eq!(sanitize_finite_clamped(-5.0, 0.0, -1.0, 1.0), -1.0);
    }

    #[test]
    fn sanitize_finite_clamped_returns_fallback_for_non_finite() {
        assert_eq!(sanitize_finite_clamped(f32::NAN, 0.0, -1.0, 1.0), 0.0);
        assert_eq!(sanitize_finite_clamped(f32::INFINITY, 0.5, 0.0, 1.0), 0.5);
    }

    #[test]
    fn sanitize_finite_min_enforces_floor() {
        assert_eq!(sanitize_finite_min(5.0, 1.0, 0.0), 5.0);
        assert_eq!(sanitize_finite_min(-1.0, 1.0, 0.0), 0.0);
        assert_eq!(sanitize_finite_min(f32::NAN, 1.0, 0.0), 1.0);
    }

    #[test]
    fn sanitize_finite_max_enforces_ceiling() {
        assert_eq!(sanitize_finite_max(-5.0, 0.0, 0.0), -5.0);
        assert_eq!(sanitize_finite_max(3.0, 0.0, 0.0), 0.0);
        assert_eq!(sanitize_finite_max(f32::NAN, -1.0, 0.0), -1.0);
    }

    #[test]
    fn sanitize_channels_enforces_minimum_of_one() {
        assert_eq!(sanitize_channels(0), 1);
        assert_eq!(sanitize_channels(1), 1);
        assert_eq!(sanitize_channels(2), 2);
    }

    #[test]
    fn sanitize_sample_rate_enforces_minimum_of_one() {
        assert_eq!(sanitize_sample_rate(0), 1);
        assert_eq!(sanitize_sample_rate(1), 1);
        assert_eq!(sanitize_sample_rate(44_100), 44_100);
    }

    #[test]
    fn sanitize_freq_clamps_to_nyquist_range() {
        assert_eq!(sanitize_freq(1_000, 48_000), 1_000);
        assert_eq!(sanitize_freq(0, 48_000), 1);
        assert_eq!(sanitize_freq(100_000, 48_000), 23_999);
    }

    #[test]
    fn sanitize_freq_handles_low_sample_rates() {
        assert_eq!(sanitize_freq(100, 2), 1);
        assert_eq!(sanitize_freq(100, 0), 1);
    }
}
