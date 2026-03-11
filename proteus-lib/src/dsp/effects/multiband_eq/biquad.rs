//! Biquad filter primitives and runtime EQ state for the multiband EQ.

use std::f32::consts::PI;

#[derive(Clone, Copy, Debug)]
pub(super) struct EqPointParams {
    pub(super) freq_hz: u32,
    pub(super) q: f32,
    pub(super) gain_db: f32,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum LowEdgeParams {
    HighPass { freq_hz: u32, q: f32 },
    LowShelf { freq_hz: u32, q: f32, gain_db: f32 },
}

#[derive(Clone, Copy, Debug)]
pub(super) enum HighEdgeParams {
    LowPass { freq_hz: u32, q: f32 },
    HighShelf { freq_hz: u32, q: f32, gain_db: f32 },
}

#[derive(Clone, Debug)]
pub(super) struct MultibandEqState {
    sample_rate: u32,
    pub(super) channels: usize,
    points_params: Vec<EqPointParams>,
    low_edge_params: Option<LowEdgeParams>,
    high_edge_params: Option<HighEdgeParams>,
    pub(super) points: Vec<Biquad>,
    pub(super) low_edge: Option<Biquad>,
    pub(super) high_edge: Option<Biquad>,
}

impl MultibandEqState {
    pub(super) fn new(
        sample_rate: u32,
        channels: usize,
        points_params: Vec<EqPointParams>,
        low_edge_params: Option<LowEdgeParams>,
        high_edge_params: Option<HighEdgeParams>,
    ) -> Self {
        let low_edge = low_edge_params.map(|params| match params {
            LowEdgeParams::HighPass { freq_hz, q } => {
                Biquad::new(sample_rate, channels, BiquadDesign::HighPass { freq_hz, q })
            }
            LowEdgeParams::LowShelf { freq_hz, q, gain_db } => Biquad::new(
                sample_rate,
                channels,
                BiquadDesign::LowShelf { freq_hz, q, gain_db },
            ),
        });

        let points = points_params
            .iter()
            .map(|params| {
                Biquad::new(
                    sample_rate,
                    channels,
                    BiquadDesign::Peaking {
                        freq_hz: params.freq_hz,
                        q: params.q,
                        gain_db: params.gain_db,
                    },
                )
            })
            .collect();

        let high_edge = high_edge_params.map(|params| match params {
            HighEdgeParams::LowPass { freq_hz, q } => {
                Biquad::new(sample_rate, channels, BiquadDesign::LowPass { freq_hz, q })
            }
            HighEdgeParams::HighShelf { freq_hz, q, gain_db } => Biquad::new(
                sample_rate,
                channels,
                BiquadDesign::HighShelf { freq_hz, q, gain_db },
            ),
        });

        Self {
            sample_rate,
            channels,
            points_params,
            low_edge_params,
            high_edge_params,
            points,
            low_edge,
            high_edge,
        }
    }

    pub(super) fn matches(
        &self,
        sample_rate: u32,
        channels: usize,
        points_params: &[EqPointParams],
        low_edge_params: &Option<LowEdgeParams>,
        high_edge_params: &Option<HighEdgeParams>,
    ) -> bool {
        self.sample_rate == sample_rate
            && self.channels == channels
            && eq_point_params_vec_equal(&self.points_params, points_params)
            && low_edge_params_equal(self.low_edge_params, *low_edge_params)
            && high_edge_params_equal(self.high_edge_params, *high_edge_params)
    }

    pub(super) fn reset(&mut self) {
        for point in &mut self.points {
            point.reset();
        }
        if let Some(filter) = self.low_edge.as_mut() {
            filter.reset();
        }
        if let Some(filter) = self.high_edge.as_mut() {
            filter.reset();
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct BiquadCoefficients {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

#[derive(Clone, Copy, Debug)]
enum BiquadDesign {
    Peaking { freq_hz: u32, q: f32, gain_db: f32 },
    LowPass { freq_hz: u32, q: f32 },
    HighPass { freq_hz: u32, q: f32 },
    LowShelf { freq_hz: u32, q: f32, gain_db: f32 },
    HighShelf { freq_hz: u32, q: f32, gain_db: f32 },
}

#[derive(Clone, Debug)]
pub(super) struct Biquad {
    coeffs: BiquadCoefficients,
    x_n1: Vec<f32>,
    x_n2: Vec<f32>,
    y_n1: Vec<f32>,
    y_n2: Vec<f32>,
}

impl Biquad {
    fn new(sample_rate: u32, channels: usize, design: BiquadDesign) -> Self {
        let channels = channels.max(1);
        Self {
            coeffs: coefficients(sample_rate, design),
            x_n1: vec![0.0; channels],
            x_n2: vec![0.0; channels],
            y_n1: vec![0.0; channels],
            y_n2: vec![0.0; channels],
        }
    }

    pub(super) fn process_sample(&mut self, channel: usize, sample: f32) -> f32 {
        let y = self.coeffs.b0 * sample
            + self.coeffs.b1 * self.x_n1[channel]
            + self.coeffs.b2 * self.x_n2[channel]
            - self.coeffs.a1 * self.y_n1[channel]
            - self.coeffs.a2 * self.y_n2[channel];

        self.x_n2[channel] = self.x_n1[channel];
        self.x_n1[channel] = sample;
        self.y_n2[channel] = self.y_n1[channel];
        self.y_n1[channel] = y;

        y
    }

    fn reset(&mut self) {
        self.x_n1.fill(0.0);
        self.x_n2.fill(0.0);
        self.y_n1.fill(0.0);
        self.y_n2.fill(0.0);
    }
}

fn coefficients(sample_rate: u32, design: BiquadDesign) -> BiquadCoefficients {
    match design {
        BiquadDesign::Peaking { freq_hz, q, gain_db } => {
            peaking_coefficients(sample_rate, freq_hz, q, gain_db)
        }
        BiquadDesign::LowPass { freq_hz, q } => low_pass_coefficients(sample_rate, freq_hz, q),
        BiquadDesign::HighPass { freq_hz, q } => high_pass_coefficients(sample_rate, freq_hz, q),
        BiquadDesign::LowShelf { freq_hz, q, gain_db } => {
            low_shelf_coefficients(sample_rate, freq_hz, q, gain_db)
        }
        BiquadDesign::HighShelf { freq_hz, q, gain_db } => {
            high_shelf_coefficients(sample_rate, freq_hz, q, gain_db)
        }
    }
}

fn peaking_coefficients(
    sample_rate: u32,
    freq_hz: u32,
    q: f32,
    gain_db: f32,
) -> BiquadCoefficients {
    let w0 = 2.0 * PI * freq_hz as f32 / sample_rate as f32;
    let cos_w0 = w0.cos();
    let alpha = w0.sin() / (2.0 * q);
    let amplitude = 10.0_f32.powf(gain_db / 40.0);

    let b0 = 1.0 + alpha * amplitude;
    let b1 = -2.0 * cos_w0;
    let b2 = 1.0 - alpha * amplitude;
    let a0 = 1.0 + alpha / amplitude;
    let a1 = -2.0 * cos_w0;
    let a2 = 1.0 - alpha / amplitude;

    normalized_coefficients(b0, b1, b2, a0, a1, a2)
}

fn low_pass_coefficients(sample_rate: u32, freq_hz: u32, q: f32) -> BiquadCoefficients {
    let w0 = 2.0 * PI * freq_hz as f32 / sample_rate as f32;
    let cos_w0 = w0.cos();
    let alpha = w0.sin() / (2.0 * q);

    let b1 = 1.0 - cos_w0;
    let b0 = b1 / 2.0;
    let b2 = b0;
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cos_w0;
    let a2 = 1.0 - alpha;

    normalized_coefficients(b0, b1, b2, a0, a1, a2)
}

fn high_pass_coefficients(sample_rate: u32, freq_hz: u32, q: f32) -> BiquadCoefficients {
    let w0 = 2.0 * PI * freq_hz as f32 / sample_rate as f32;
    let cos_w0 = w0.cos();
    let alpha = w0.sin() / (2.0 * q);

    let b0 = (1.0 + cos_w0) / 2.0;
    let b1 = -1.0 - cos_w0;
    let b2 = b0;
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cos_w0;
    let a2 = 1.0 - alpha;

    normalized_coefficients(b0, b1, b2, a0, a1, a2)
}

fn low_shelf_coefficients(
    sample_rate: u32,
    freq_hz: u32,
    q: f32,
    gain_db: f32,
) -> BiquadCoefficients {
    let w0 = 2.0 * PI * freq_hz as f32 / sample_rate as f32;
    let cos_w0 = w0.cos();
    let alpha = w0.sin() / (2.0 * q);
    let amplitude = 10.0_f32.powf(gain_db / 40.0);
    let sqrt_amplitude = amplitude.sqrt();

    let b0 =
        amplitude * ((amplitude + 1.0) - (amplitude - 1.0) * cos_w0 + 2.0 * sqrt_amplitude * alpha);
    let b1 = 2.0 * amplitude * ((amplitude - 1.0) - (amplitude + 1.0) * cos_w0);
    let b2 =
        amplitude * ((amplitude + 1.0) - (amplitude - 1.0) * cos_w0 - 2.0 * sqrt_amplitude * alpha);
    let a0 = (amplitude + 1.0) + (amplitude - 1.0) * cos_w0 + 2.0 * sqrt_amplitude * alpha;
    let a1 = -2.0 * ((amplitude - 1.0) + (amplitude + 1.0) * cos_w0);
    let a2 = (amplitude + 1.0) + (amplitude - 1.0) * cos_w0 - 2.0 * sqrt_amplitude * alpha;

    normalized_coefficients(b0, b1, b2, a0, a1, a2)
}

fn high_shelf_coefficients(
    sample_rate: u32,
    freq_hz: u32,
    q: f32,
    gain_db: f32,
) -> BiquadCoefficients {
    let w0 = 2.0 * PI * freq_hz as f32 / sample_rate as f32;
    let cos_w0 = w0.cos();
    let alpha = w0.sin() / (2.0 * q);
    let amplitude = 10.0_f32.powf(gain_db / 40.0);
    let sqrt_amplitude = amplitude.sqrt();

    let b0 =
        amplitude * ((amplitude + 1.0) + (amplitude - 1.0) * cos_w0 + 2.0 * sqrt_amplitude * alpha);
    let b1 = -2.0 * amplitude * ((amplitude - 1.0) + (amplitude + 1.0) * cos_w0);
    let b2 =
        amplitude * ((amplitude + 1.0) + (amplitude - 1.0) * cos_w0 - 2.0 * sqrt_amplitude * alpha);
    let a0 = (amplitude + 1.0) - (amplitude - 1.0) * cos_w0 + 2.0 * sqrt_amplitude * alpha;
    let a1 = 2.0 * ((amplitude - 1.0) - (amplitude + 1.0) * cos_w0);
    let a2 = (amplitude + 1.0) - (amplitude - 1.0) * cos_w0 - 2.0 * sqrt_amplitude * alpha;

    normalized_coefficients(b0, b1, b2, a0, a1, a2)
}

fn normalized_coefficients(
    b0: f32,
    b1: f32,
    b2: f32,
    a0: f32,
    a1: f32,
    a2: f32,
) -> BiquadCoefficients {
    BiquadCoefficients {
        b0: b0 / a0,
        b1: b1 / a0,
        b2: b2 / a0,
        a1: a1 / a0,
        a2: a2 / a0,
    }
}

fn eq_point_params_vec_equal(left: &[EqPointParams], right: &[EqPointParams]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right.iter())
            .all(|(l, r)| eq_point_params_equal(*l, *r))
}

fn eq_point_params_equal(left: EqPointParams, right: EqPointParams) -> bool {
    left.freq_hz == right.freq_hz
        && (left.q - right.q).abs() < f32::EPSILON
        && (left.gain_db - right.gain_db).abs() < f32::EPSILON
}

fn low_edge_params_equal(left: Option<LowEdgeParams>, right: Option<LowEdgeParams>) -> bool {
    match (left, right) {
        (None, None) => true,
        (
            Some(LowEdgeParams::HighPass { freq_hz: lf, q: lq }),
            Some(LowEdgeParams::HighPass { freq_hz: rf, q: rq }),
        ) => lf == rf && (lq - rq).abs() < f32::EPSILON,
        (
            Some(LowEdgeParams::LowShelf { freq_hz: lf, q: lq, gain_db: lg }),
            Some(LowEdgeParams::LowShelf { freq_hz: rf, q: rq, gain_db: rg }),
        ) => lf == rf && (lq - rq).abs() < f32::EPSILON && (lg - rg).abs() < f32::EPSILON,
        _ => false,
    }
}

fn high_edge_params_equal(left: Option<HighEdgeParams>, right: Option<HighEdgeParams>) -> bool {
    match (left, right) {
        (None, None) => true,
        (
            Some(HighEdgeParams::LowPass { freq_hz: lf, q: lq }),
            Some(HighEdgeParams::LowPass { freq_hz: rf, q: rq }),
        ) => lf == rf && (lq - rq).abs() < f32::EPSILON,
        (
            Some(HighEdgeParams::HighShelf { freq_hz: lf, q: lq, gain_db: lg }),
            Some(HighEdgeParams::HighShelf { freq_hz: rf, q: rq, gain_db: rg }),
        ) => lf == rf && (lq - rq).abs() < f32::EPSILON && (lg - rg).abs() < f32::EPSILON,
        _ => false,
    }
}
