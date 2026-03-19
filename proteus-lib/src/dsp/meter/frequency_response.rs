//! Pure analytical frequency-response helpers.

use crate::dsp::guardrails::sanitize_sample_rate;

use super::FrequencyResponsePoint;

const MIN_RESPONSE_FREQ_HZ: f32 = 20.0;
const MIN_MAGNITUDE: f32 = 1.0e-12;

pub(crate) fn build_log_spaced_curve<F>(
    sample_rate: u32,
    num_points: usize,
    mut response_db: F,
) -> Vec<FrequencyResponsePoint>
where
    F: FnMut(f32) -> f32,
{
    response_probe_frequencies(sample_rate, num_points)
        .into_iter()
        .map(|freq_hz| FrequencyResponsePoint {
            freq_hz,
            gain_db: response_db(freq_hz),
        })
        .collect()
}

pub(crate) fn identity_curve(sample_rate: u32, num_points: usize) -> Vec<FrequencyResponsePoint> {
    build_log_spaced_curve(sample_rate, num_points, |_| 0.0)
}

pub(crate) fn sum_curves(curves: &[Vec<FrequencyResponsePoint>]) -> Vec<FrequencyResponsePoint> {
    let Some(first) = curves.first() else {
        return Vec::new();
    };

    first
        .iter()
        .enumerate()
        .map(|(index, point)| FrequencyResponsePoint {
            freq_hz: point.freq_hz,
            gain_db: curves.iter().map(|curve| curve[index].gain_db).sum(),
        })
        .collect()
}

pub(crate) fn magnitude_to_db(magnitude: f32) -> f32 {
    20.0 * magnitude.max(MIN_MAGNITUDE).log10()
}

fn response_probe_frequencies(sample_rate: u32, num_points: usize) -> Vec<f32> {
    let sample_rate = sanitize_sample_rate(sample_rate);
    let nyquist_hz = (sample_rate as f32 * 0.5).max(1.0);
    let start_hz = MIN_RESPONSE_FREQ_HZ.min(nyquist_hz).max(1.0);

    if num_points <= 1 {
        return vec![start_hz];
    }

    let start_log = start_hz.ln();
    let end_log = nyquist_hz.ln();
    let step = (end_log - start_log) / (num_points - 1) as f32;
    (0..num_points)
        .map(|index| (start_log + step * index as f32).exp())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{build_log_spaced_curve, identity_curve};

    #[test]
    fn identity_curve_builds_requested_number_of_points() {
        let curve = identity_curve(48_000, 8);
        assert_eq!(curve.len(), 8);
        assert!(curve.iter().all(|point| point.gain_db.abs() < 1.0e-6));
    }

    #[test]
    fn log_spaced_curve_uses_monotonic_frequencies() {
        let curve = build_log_spaced_curve(48_000, 16, |_| 0.0);
        assert_eq!(curve.len(), 16);
        assert!(curve
            .windows(2)
            .all(|pair| pair[0].freq_hz < pair[1].freq_hz));
    }
}
