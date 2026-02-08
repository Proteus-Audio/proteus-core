//! Internal biquad filter helpers derived from rodio BLT filters.

use std::f32::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BiquadKind {
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
pub(super) struct BiquadState {
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
    pub(super) fn new(kind: BiquadKind, sample_rate: u32, channels: usize, freq: u32, q: f32) -> Self {
        let freq = sanitize_freq(freq, sample_rate);
        let q = sanitize_q(q);
        let coeffs = coefficients(kind, sample_rate, freq, q);
        let channels = channels.max(1);
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

    pub(super) fn matches(&self, kind: BiquadKind, sample_rate: u32, channels: usize, freq: u32, q: f32) -> bool {
        let freq = sanitize_freq(freq, sample_rate);
        let q = sanitize_q(q);
        self.kind == kind
            && self.sample_rate == sample_rate
            && self.channels == channels.max(1)
            && self.freq == freq
            && (self.q - q).abs() < f32::EPSILON
    }

    pub(super) fn process(&mut self, samples: &[f32]) -> Vec<f32> {
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

    pub(super) fn reset(&mut self) {
        self.x_n1.fill(0.0);
        self.x_n2.fill(0.0);
        self.y_n1.fill(0.0);
        self.y_n2.fill(0.0);
    }
}

fn sanitize_freq(freq: u32, sample_rate: u32) -> u32 {
    let nyquist = sample_rate / 2;
    if nyquist <= 1 {
        return 1;
    }
    freq.clamp(1, nyquist.saturating_sub(1).max(1))
}

fn sanitize_q(q: f32) -> f32 {
    if !q.is_finite() {
        return 0.5;
    }
    q.clamp(0.1, 10.0)
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
