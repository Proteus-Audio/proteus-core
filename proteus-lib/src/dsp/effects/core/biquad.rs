//! Internal biquad filter helpers derived from rodio BLT filters.

use std::f32::consts::PI;

use crate::dsp::guardrails::{sanitize_channels, sanitize_finite_clamped, sanitize_freq};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BiquadKind {
    LowPass,
    HighPass,
}

/// Number of samples over which biquad coefficients are ramped by default.
const DEFAULT_COEFF_RAMP_SAMPLES: usize = 240;

#[derive(Clone, Copy, Debug)]
pub(super) struct BiquadCoefficients {
    pub(super) b0: f32,
    pub(super) b1: f32,
    pub(super) b2: f32,
    pub(super) a1: f32,
    pub(super) a2: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct BiquadState {
    kind: BiquadKind,
    sample_rate: u32,
    channels: usize,
    freq: u32,
    q: f32,
    coeffs: BiquadCoefficients,
    target_coeffs: Option<BiquadCoefficients>,
    coeff_deltas: Option<CoefficientDeltas>,
    ramp_remaining: usize,
    x_n1: Vec<f32>,
    x_n2: Vec<f32>,
    y_n1: Vec<f32>,
    y_n2: Vec<f32>,
}

#[derive(Clone, Copy, Debug)]
struct CoefficientDeltas {
    d_b0: f32,
    d_b1: f32,
    d_b2: f32,
    d_a1: f32,
    d_a2: f32,
}

impl BiquadState {
    pub(crate) fn new(
        kind: BiquadKind,
        sample_rate: u32,
        channels: usize,
        freq: u32,
        q: f32,
    ) -> Self {
        let freq = sanitize_freq(freq, sample_rate);
        let q = sanitize_finite_clamped(q, 0.5, 0.1, 10.0);
        let coeffs = coefficients(kind, sample_rate, freq, q);
        let channels = sanitize_channels(channels);
        Self {
            kind,
            sample_rate,
            channels,
            freq,
            q,
            coeffs,
            target_coeffs: None,
            coeff_deltas: None,
            ramp_remaining: 0,
            x_n1: vec![0.0; channels],
            x_n2: vec![0.0; channels],
            y_n1: vec![0.0; channels],
            y_n2: vec![0.0; channels],
        }
    }

    /// Check whether structural parameters (kind, sample_rate, channels) match.
    ///
    /// Returns `true` when no full reconstruction is needed. Freq/Q mismatches
    /// are handled by [`update_coefficients`] instead.
    pub(crate) fn matches_structure(
        &self,
        kind: BiquadKind,
        sample_rate: u32,
        channels: usize,
    ) -> bool {
        self.kind == kind
            && self.sample_rate == sample_rate
            && self.channels == sanitize_channels(channels)
    }

    /// Legacy full match including freq and Q. Still used by callers that
    /// want to know if the state is already exactly up-to-date.
    #[cfg(test)]
    pub(crate) fn matches(
        &self,
        kind: BiquadKind,
        sample_rate: u32,
        channels: usize,
        freq: u32,
        q: f32,
    ) -> bool {
        let freq = sanitize_freq(freq, sample_rate);
        let q = sanitize_finite_clamped(q, 0.5, 0.1, 10.0);
        self.matches_structure(kind, sample_rate, channels)
            && self.freq == freq
            && (self.q - q).abs() < f32::EPSILON
            && self.ramp_remaining == 0
    }

    /// Smoothly transition to new freq/Q by ramping coefficients over
    /// `ramp_samples` samples, preserving the delay line state.
    pub(crate) fn update_coefficients(&mut self, freq: u32, q: f32, ramp_samples: usize) {
        let freq = sanitize_freq(freq, self.sample_rate);
        let q = sanitize_finite_clamped(q, 0.5, 0.1, 10.0);

        if self.freq == freq && (self.q - q).abs() < f32::EPSILON && self.ramp_remaining == 0 {
            return;
        }

        self.freq = freq;
        self.q = q;
        let target = coefficients(self.kind, self.sample_rate, freq, q);

        let ramp = if ramp_samples == 0 {
            DEFAULT_COEFF_RAMP_SAMPLES
        } else {
            ramp_samples
        };

        let inv = 1.0 / ramp as f32;
        self.target_coeffs = Some(target);
        self.coeff_deltas = Some(CoefficientDeltas {
            d_b0: (target.b0 - self.coeffs.b0) * inv,
            d_b1: (target.b1 - self.coeffs.b1) * inv,
            d_b2: (target.b2 - self.coeffs.b2) * inv,
            d_a1: (target.a1 - self.coeffs.a1) * inv,
            d_a2: (target.a2 - self.coeffs.a2) * inv,
        });
        self.ramp_remaining = ramp;
    }

    /// Advance coefficient ramp by one sample.
    #[inline]
    fn advance_coefficients(&mut self) {
        if self.ramp_remaining == 0 {
            return;
        }
        self.ramp_remaining -= 1;
        if self.ramp_remaining == 0 {
            if let Some(target) = self.target_coeffs.take() {
                self.coeffs = target;
            }
            self.coeff_deltas = None;
        } else if let Some(d) = &self.coeff_deltas {
            self.coeffs.b0 += d.d_b0;
            self.coeffs.b1 += d.d_b1;
            self.coeffs.b2 += d.d_b2;
            self.coeffs.a1 += d.d_a1;
            self.coeffs.a2 += d.d_a2;
        }
    }

    pub(crate) fn process(&mut self, samples: &[f32]) -> Vec<f32> {
        if samples.is_empty() {
            return Vec::new();
        }

        let mut out = Vec::with_capacity(samples.len());
        let channels = self.channels;
        let ramping = self.ramp_remaining > 0;
        for (idx, &sample) in samples.iter().enumerate() {
            let ch = idx % channels;
            let result = self.coeffs.b0 * sample
                + self.coeffs.b1 * self.x_n1[ch]
                + self.coeffs.b2 * self.x_n2[ch]
                - self.coeffs.a1 * self.y_n1[ch]
                - self.coeffs.a2 * self.y_n2[ch];

            self.x_n2[ch] = self.x_n1[ch];
            self.x_n1[ch] = sample;
            self.y_n2[ch] = self.y_n1[ch];
            self.y_n1[ch] = result;

            out.push(result);

            if ramping && ch == channels - 1 {
                self.advance_coefficients();
            }
        }

        out
    }

    /// Process interleaved samples through the biquad filter, appending output to `output`.
    ///
    /// The caller must clear `output` before calling if a fresh result is needed.
    /// No allocation is performed when `output` already has sufficient capacity.
    pub(crate) fn process_into(&mut self, samples: &[f32], output: &mut Vec<f32>) {
        if samples.is_empty() {
            return;
        }
        let channels = self.channels;
        let ramping = self.ramp_remaining > 0;
        for (idx, &sample) in samples.iter().enumerate() {
            let ch = idx % channels;
            let result = self.coeffs.b0 * sample
                + self.coeffs.b1 * self.x_n1[ch]
                + self.coeffs.b2 * self.x_n2[ch]
                - self.coeffs.a1 * self.y_n1[ch]
                - self.coeffs.a2 * self.y_n2[ch];

            self.x_n2[ch] = self.x_n1[ch];
            self.x_n1[ch] = sample;
            self.y_n2[ch] = self.y_n1[ch];
            self.y_n1[ch] = result;

            output.push(result);

            if ramping && ch == channels - 1 {
                self.advance_coefficients();
            }
        }
    }

    pub(crate) fn reset(&mut self) {
        self.x_n1.fill(0.0);
        self.x_n2.fill(0.0);
        self.y_n1.fill(0.0);
        self.y_n2.fill(0.0);
        self.target_coeffs = None;
        self.coeff_deltas = None;
        self.ramp_remaining = 0;
    }
}

fn coefficients(kind: BiquadKind, sample_rate: u32, freq: u32, q: f32) -> BiquadCoefficients {
    let w0 = 2.0 * PI * freq as f32 / sample_rate as f32;
    let cos_w0 = w0.cos();
    let alpha = w0.sin() / (2.0 * q);

    match kind {
        BiquadKind::LowPass => {
            let b1 = 1.0 - cos_w0;
            let b0 = b1 / 2.0;
            let b2 = b0;
            let a0 = 1.0 + alpha;
            let a1 = -2.0 * cos_w0;
            let a2 = 1.0 - alpha;

            BiquadCoefficients {
                b0: b0 / a0,
                b1: b1 / a0,
                b2: b2 / a0,
                a1: a1 / a0,
                a2: a2 / a0,
            }
        }
        BiquadKind::HighPass => {
            let b0 = (1.0 + cos_w0) / 2.0;
            let b1 = -1.0 - cos_w0;
            let b2 = b0;
            let a0 = 1.0 + alpha;
            let a1 = -2.0 * cos_w0;
            let a2 = 1.0 - alpha;

            BiquadCoefficients {
                b0: b0 / a0,
                b1: b1 / a0,
                b2: b2 / a0,
                a1: a1 / a0,
                a2: a2 / a0,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn biquad_process_returns_matching_length() {
        let mut state = BiquadState::new(BiquadKind::LowPass, 48_000, 2, 1_200, 0.707);
        let input = vec![0.0_f32, 1.0, 0.5, -0.5, -1.0, 0.25];
        let output = state.process(&input);
        assert_eq!(output.len(), input.len());
    }

    #[test]
    fn biquad_matches_uses_sanitized_values() {
        let state = BiquadState::new(BiquadKind::HighPass, 48_000, 1, 200_000, f32::NAN);
        assert!(state.matches(BiquadKind::HighPass, 48_000, 1, 200_000, f32::NAN));
    }

    #[test]
    fn sanitize_helpers_clamp_invalid_input() {
        assert_eq!(sanitize_freq(0, 48_000), 1);
        assert_eq!(sanitize_finite_clamped(f32::INFINITY, 0.5, 0.1, 10.0), 0.5);
        assert_eq!(sanitize_finite_clamped(0.01, 0.5, 0.1, 10.0), 0.1);
        assert_eq!(sanitize_finite_clamped(20.0, 0.5, 0.1, 10.0), 10.0);
    }

    #[test]
    fn biquad_process_into_matches_process() {
        let mut state_a = BiquadState::new(BiquadKind::LowPass, 48_000, 2, 1_200, 0.707);
        let mut state_b = BiquadState::new(BiquadKind::LowPass, 48_000, 2, 1_200, 0.707);
        let input = vec![0.0_f32, 1.0, 0.5, -0.5, -1.0, 0.25];
        let expected = state_a.process(&input);
        let mut got = Vec::new();
        state_b.process_into(&input, &mut got);
        assert_eq!(got, expected);
    }

    #[test]
    fn biquad_reset_clears_state_history() {
        let mut state = BiquadState::new(BiquadKind::LowPass, 48_000, 2, 1_200, 0.707);
        let _ = state.process(&[1.0, -1.0, 0.5, -0.5]);
        state.reset();
        let output = state.process(&[0.0, 0.0]);
        assert_eq!(output.len(), 2);
        assert!(output.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn coefficient_updates_preserve_delay_line_better_than_reset() {
        let mut smoothed = BiquadState::new(BiquadKind::LowPass, 48_000, 1, 400, 0.7);
        let mut reset = BiquadState::new(BiquadKind::LowPass, 48_000, 1, 400, 0.7);

        let input = (0..256)
            .map(|index| {
                let phase = 2.0 * std::f32::consts::PI * 440.0 * index as f32 / 48_000.0;
                phase.sin()
            })
            .collect::<Vec<_>>();

        let initial = smoothed.process(&input);
        let previous = *initial.last().unwrap();
        let _ = reset.process(&input);

        smoothed.update_coefficients(4_000, 0.9, 240);
        let smoothed_output = smoothed.process(&input[..64]);

        reset = BiquadState::new(BiquadKind::LowPass, 48_000, 1, 4_000, 0.9);
        let reset_output = reset.process(&input[..64]);

        let smoothed_jump = (smoothed_output[0] - previous).abs();
        let reset_jump = (reset_output[0] - previous).abs();
        assert!(smoothed_jump < reset_jump);
    }

    #[test]
    fn fast_coefficient_sweeps_remain_finite() {
        let mut state = BiquadState::new(BiquadKind::HighPass, 48_000, 2, 200, 0.7);
        let input = vec![0.25_f32; 128];
        for step in 0..64 {
            state.update_coefficients(200 + step * 200, 0.4 + step as f32 * 0.01, 4);
            let output = state.process(&input);
            assert!(output.iter().all(|sample| sample.is_finite()));
        }
    }
}
