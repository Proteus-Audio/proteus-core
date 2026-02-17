//! Multiband parametric EQ effect.
//!
//! This effect applies a configurable chain of parametric EQ points plus
//! optional edge-shaping filters for low and high frequency boundaries.

use std::f32::consts::PI;

use serde::{Deserialize, Serialize};

use super::EffectContext;

const DEFAULT_LOW_FREQ_HZ: u32 = 120;
const DEFAULT_MID_FREQ_HZ: u32 = 1_000;
const DEFAULT_HIGH_FREQ_HZ: u32 = 8_000;
const DEFAULT_Q: f32 = 0.8;
const DEFAULT_GAIN_DB: f32 = 0.0;
const MIN_Q: f32 = 0.1;
const MAX_Q: f32 = 10.0;
const MIN_GAIN_DB: f32 = -24.0;
const MAX_GAIN_DB: f32 = 24.0;

/// Serialized configuration for a single parametric EQ point.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EqPointSettings {
    pub freq_hz: u32,
    pub q: f32,
    pub gain_db: f32,
}

impl EqPointSettings {
    /// Create an EQ point.
    pub fn new(freq_hz: u32, q: f32, gain_db: f32) -> Self {
        Self {
            freq_hz,
            q,
            gain_db,
        }
    }
}

impl Default for EqPointSettings {
    fn default() -> Self {
        Self {
            freq_hz: DEFAULT_MID_FREQ_HZ,
            q: DEFAULT_Q,
            gain_db: DEFAULT_GAIN_DB,
        }
    }
}

/// Optional low-edge shaping.
///
/// `HighPass` removes low-end energy below the cutoff.
/// `LowShelf` boosts/cuts the low-end around the center frequency.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LowEdgeFilterSettings {
    HighPass { freq_hz: u32, q: f32 },
    LowShelf { freq_hz: u32, q: f32, gain_db: f32 },
}

/// Optional high-edge shaping.
///
/// `LowPass` removes high-end energy above the cutoff.
/// `HighShelf` boosts/cuts the high-end around the center frequency.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HighEdgeFilterSettings {
    LowPass { freq_hz: u32, q: f32 },
    HighShelf { freq_hz: u32, q: f32, gain_db: f32 },
}

/// Serialized configuration for multiband EQ.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MultibandEqSettings {
    #[serde(alias = "bands", alias = "eq_points")]
    pub points: Vec<EqPointSettings>,
    pub low_edge: Option<LowEdgeFilterSettings>,
    pub high_edge: Option<HighEdgeFilterSettings>,
}

impl MultibandEqSettings {
    /// Create multiband EQ settings.
    pub fn new(
        points: Vec<EqPointSettings>,
        low_edge: Option<LowEdgeFilterSettings>,
        high_edge: Option<HighEdgeFilterSettings>,
    ) -> Self {
        Self {
            points,
            low_edge,
            high_edge,
        }
    }
}

impl Default for MultibandEqSettings {
    fn default() -> Self {
        Self {
            points: vec![
                EqPointSettings::new(DEFAULT_LOW_FREQ_HZ, DEFAULT_Q, DEFAULT_GAIN_DB),
                EqPointSettings::new(DEFAULT_MID_FREQ_HZ, DEFAULT_Q, DEFAULT_GAIN_DB),
                EqPointSettings::new(DEFAULT_HIGH_FREQ_HZ, DEFAULT_Q, DEFAULT_GAIN_DB),
            ],
            low_edge: None,
            high_edge: None,
        }
    }
}

/// Configured multiband EQ effect with runtime state.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MultibandEqEffect {
    pub enabled: bool,
    #[serde(flatten)]
    pub settings: MultibandEqSettings,
    #[serde(skip)]
    state: Option<MultibandEqState>,
}

impl std::fmt::Debug for MultibandEqEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultibandEqEffect")
            .field("enabled", &self.enabled)
            .field("settings", &self.settings)
            .finish()
    }
}

impl Default for MultibandEqEffect {
    fn default() -> Self {
        Self {
            enabled: false,
            settings: MultibandEqSettings::default(),
            state: None,
        }
    }
}

impl MultibandEqEffect {
    /// Process interleaved samples through the multiband EQ.
    ///
    /// # Arguments
    /// - `samples`: Interleaved input samples.
    /// - `context`: Environment details (sample rate, channels, etc.).
    /// - `drain`: Unused for this effect.
    ///
    /// # Returns
    /// Processed interleaved samples.
    pub fn process(&mut self, samples: &[f32], context: &EffectContext, _drain: bool) -> Vec<f32> {
        if !self.enabled {
            return samples.to_vec();
        }

        self.ensure_state(context);
        let Some(state) = self.state.as_mut() else {
            return samples.to_vec();
        };

        if samples.is_empty() {
            return Vec::new();
        }

        let channels = state.channels;
        let mut output = Vec::with_capacity(samples.len());

        for (idx, &sample) in samples.iter().enumerate() {
            let ch = idx % channels;
            let mut y = sample;

            if let Some(filter) = state.low_edge.as_mut() {
                y = filter.process_sample(ch, y);
            }

            for point in &mut state.points {
                y = point.process_sample(ch, y);
            }

            if let Some(filter) = state.high_edge.as_mut() {
                y = filter.process_sample(ch, y);
            }

            output.push(y);
        }

        output
    }

    /// Reset any internal state held by the multiband EQ.
    pub fn reset_state(&mut self) {
        if let Some(state) = self.state.as_mut() {
            state.reset();
        }
        self.state = None;
    }

    fn ensure_state(&mut self, context: &EffectContext) {
        let channels = context.channels.max(1);
        let points = self
            .settings
            .points
            .iter()
            .map(|point| EqPointParams {
                freq_hz: sanitize_freq(point.freq_hz, context.sample_rate),
                q: sanitize_q(point.q),
                gain_db: sanitize_gain_db(point.gain_db),
            })
            .collect::<Vec<_>>();

        let low_edge = self
            .settings
            .low_edge
            .as_ref()
            .map(|edge| sanitize_low_edge(edge, context.sample_rate));
        let high_edge = self
            .settings
            .high_edge
            .as_ref()
            .map(|edge| sanitize_high_edge(edge, context.sample_rate));

        let needs_reset = self
            .state
            .as_ref()
            .map(|state| {
                state.matches(
                    context.sample_rate,
                    channels,
                    &points,
                    &low_edge,
                    &high_edge,
                )
            })
            .map(|matches| !matches)
            .unwrap_or(true);

        if needs_reset {
            self.state = Some(MultibandEqState::new(
                context.sample_rate,
                channels,
                points,
                low_edge,
                high_edge,
            ));
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct EqPointParams {
    freq_hz: u32,
    q: f32,
    gain_db: f32,
}

#[derive(Clone, Copy, Debug)]
enum LowEdgeParams {
    HighPass { freq_hz: u32, q: f32 },
    LowShelf { freq_hz: u32, q: f32, gain_db: f32 },
}

#[derive(Clone, Copy, Debug)]
enum HighEdgeParams {
    LowPass { freq_hz: u32, q: f32 },
    HighShelf { freq_hz: u32, q: f32, gain_db: f32 },
}

#[derive(Clone, Debug)]
struct MultibandEqState {
    sample_rate: u32,
    channels: usize,
    points_params: Vec<EqPointParams>,
    low_edge_params: Option<LowEdgeParams>,
    high_edge_params: Option<HighEdgeParams>,
    points: Vec<Biquad>,
    low_edge: Option<Biquad>,
    high_edge: Option<Biquad>,
}

impl MultibandEqState {
    fn new(
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
            LowEdgeParams::LowShelf {
                freq_hz,
                q,
                gain_db,
            } => Biquad::new(
                sample_rate,
                channels,
                BiquadDesign::LowShelf {
                    freq_hz,
                    q,
                    gain_db,
                },
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
            HighEdgeParams::HighShelf {
                freq_hz,
                q,
                gain_db,
            } => Biquad::new(
                sample_rate,
                channels,
                BiquadDesign::HighShelf {
                    freq_hz,
                    q,
                    gain_db,
                },
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

    fn matches(
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

    fn reset(&mut self) {
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
struct Biquad {
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

    fn process_sample(&mut self, channel: usize, sample: f32) -> f32 {
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
        BiquadDesign::Peaking {
            freq_hz,
            q,
            gain_db,
        } => peaking_coefficients(sample_rate, freq_hz, q, gain_db),
        BiquadDesign::LowPass { freq_hz, q } => low_pass_coefficients(sample_rate, freq_hz, q),
        BiquadDesign::HighPass { freq_hz, q } => high_pass_coefficients(sample_rate, freq_hz, q),
        BiquadDesign::LowShelf {
            freq_hz,
            q,
            gain_db,
        } => low_shelf_coefficients(sample_rate, freq_hz, q, gain_db),
        BiquadDesign::HighShelf {
            freq_hz,
            q,
            gain_db,
        } => high_shelf_coefficients(sample_rate, freq_hz, q, gain_db),
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

fn sanitize_low_edge(edge: &LowEdgeFilterSettings, sample_rate: u32) -> LowEdgeParams {
    match edge {
        LowEdgeFilterSettings::HighPass { freq_hz, q } => LowEdgeParams::HighPass {
            freq_hz: sanitize_freq(*freq_hz, sample_rate),
            q: sanitize_q(*q),
        },
        LowEdgeFilterSettings::LowShelf {
            freq_hz,
            q,
            gain_db,
        } => LowEdgeParams::LowShelf {
            freq_hz: sanitize_freq(*freq_hz, sample_rate),
            q: sanitize_q(*q),
            gain_db: sanitize_gain_db(*gain_db),
        },
    }
}

fn sanitize_high_edge(edge: &HighEdgeFilterSettings, sample_rate: u32) -> HighEdgeParams {
    match edge {
        HighEdgeFilterSettings::LowPass { freq_hz, q } => HighEdgeParams::LowPass {
            freq_hz: sanitize_freq(*freq_hz, sample_rate),
            q: sanitize_q(*q),
        },
        HighEdgeFilterSettings::HighShelf {
            freq_hz,
            q,
            gain_db,
        } => HighEdgeParams::HighShelf {
            freq_hz: sanitize_freq(*freq_hz, sample_rate),
            q: sanitize_q(*q),
            gain_db: sanitize_gain_db(*gain_db),
        },
    }
}

fn sanitize_freq(freq_hz: u32, sample_rate: u32) -> u32 {
    let nyquist = sample_rate / 2;
    if nyquist <= 1 {
        return 1;
    }
    freq_hz.clamp(1, nyquist.saturating_sub(1).max(1))
}

fn sanitize_q(q: f32) -> f32 {
    if !q.is_finite() {
        return DEFAULT_Q;
    }
    q.clamp(MIN_Q, MAX_Q)
}

fn sanitize_gain_db(gain_db: f32) -> f32 {
    if !gain_db.is_finite() {
        return DEFAULT_GAIN_DB;
    }
    gain_db.clamp(MIN_GAIN_DB, MAX_GAIN_DB)
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
            Some(LowEdgeParams::LowShelf {
                freq_hz: lf,
                q: lq,
                gain_db: lg,
            }),
            Some(LowEdgeParams::LowShelf {
                freq_hz: rf,
                q: rq,
                gain_db: rg,
            }),
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
            Some(HighEdgeParams::HighShelf {
                freq_hz: lf,
                q: lq,
                gain_db: lg,
            }),
            Some(HighEdgeParams::HighShelf {
                freq_hz: rf,
                q: rq,
                gain_db: rg,
            }),
        ) => lf == rf && (lq - rq).abs() < f32::EPSILON && (lg - rg).abs() < f32::EPSILON,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> EffectContext {
        EffectContext {
            sample_rate: 48_000,
            channels: 2,
            container_path: None,
            impulse_response_spec: None,
            impulse_response_tail_db: -60.0,
        }
    }

    #[test]
    fn multiband_eq_disabled_passthrough() {
        let mut effect = MultibandEqEffect::default();
        let samples = vec![0.25_f32, -0.25, 0.5, -0.5];
        let output = effect.process(&samples, &context(), false);
        assert_eq!(output, samples);
    }

    #[test]
    fn multiband_eq_points_and_edges_change_signal() {
        let mut effect = MultibandEqEffect::default();
        effect.enabled = true;
        effect.settings.points = vec![
            EqPointSettings::new(120, 0.8, 6.0),
            EqPointSettings::new(1_000, 1.2, -4.0),
            EqPointSettings::new(8_000, 0.9, 3.0),
            EqPointSettings::new(12_000, 0.7, -2.0),
        ];
        effect.settings.low_edge = Some(LowEdgeFilterSettings::HighPass {
            freq_hz: 40,
            q: 0.7,
        });
        effect.settings.high_edge = Some(HighEdgeFilterSettings::HighShelf {
            freq_hz: 10_000,
            q: 0.8,
            gain_db: 2.0,
        });

        let samples = vec![0.1_f32, -0.1, 0.2, -0.2, 0.15, -0.15, 0.3, -0.3];
        let output = effect.process(&samples, &context(), false);

        assert_eq!(output.len(), samples.len());
        assert!(output.iter().all(|value| value.is_finite()));
        assert!(output
            .iter()
            .zip(samples.iter())
            .any(|(out, input)| (*out - *input).abs() > 1e-6));
    }

    #[test]
    fn multiband_eq_deserializes_vec_points_and_edge_variants() {
        let json = r#"{
            "enabled": true,
            "points": [
                {"freq_hz": 120, "q": 0.8, "gain_db": 4.5},
                {"freq_hz": 800, "q": 1.1, "gain_db": -3.0}
            ],
            "low_edge": {"type": "low_shelf", "freq_hz": 100, "q": 0.8, "gain_db": 2.0},
            "high_edge": {"type": "low_pass", "freq_hz": 14000, "q": 0.7}
        }"#;

        let effect: MultibandEqEffect =
            serde_json::from_str(json).expect("deserialize multiband eq");
        assert_eq!(effect.settings.points.len(), 2);
        assert!(matches!(
            effect.settings.low_edge,
            Some(LowEdgeFilterSettings::LowShelf { .. })
        ));
        assert!(matches!(
            effect.settings.high_edge,
            Some(HighEdgeFilterSettings::LowPass { .. })
        ));
    }
}
