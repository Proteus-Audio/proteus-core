//! Internal biquad filter helpers derived from rodio BLT filters.

use std::f32::consts::PI;

use crate::dsp::guardrails::{sanitize_channels, sanitize_finite_clamped, sanitize_freq};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BiquadKind {
    LowPass,
    HighPass,
}

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
    x_n1: Vec<f32>,
    x_n2: Vec<f32>,
    y_n1: Vec<f32>,
    y_n2: Vec<f32>,
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
            x_n1: vec![0.0; channels],
            x_n2: vec![0.0; channels],
            y_n1: vec![0.0; channels],
            y_n2: vec![0.0; channels],
        }
    }

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
        self.kind == kind
            && self.sample_rate == sample_rate
            && self.channels == sanitize_channels(channels)
            && self.freq == freq
            && (self.q - q).abs() < f32::EPSILON
    }

    pub(crate) fn process(&mut self, samples: &[f32]) -> Vec<f32> {
        if samples.is_empty() {
            return Vec::new();
        }

        let mut out = Vec::with_capacity(samples.len());
        let channels = self.channels;
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
        }

        out
    }

    pub(crate) fn reset(&mut self) {
        self.x_n1.fill(0.0);
        self.x_n2.fill(0.0);
        self.y_n1.fill(0.0);
        self.y_n2.fill(0.0);
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
    fn biquad_reset_clears_state_history() {
        let mut state = BiquadState::new(BiquadKind::LowPass, 48_000, 2, 1_200, 0.707);
        let _ = state.process(&[1.0, -1.0, 0.5, -0.5]);
        state.reset();
        let output = state.process(&[0.0, 0.0]);
        assert_eq!(output.len(), 2);
        assert!(output.iter().all(|v| v.is_finite()));
    }
}
