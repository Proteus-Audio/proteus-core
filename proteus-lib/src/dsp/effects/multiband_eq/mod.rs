//! Multiband parametric EQ effect.
//!
//! This effect applies a configurable chain of parametric EQ points plus
//! optional edge-shaping filters for low and high frequency boundaries.
//!
//! Biquad filter primitives and coefficient computation live in [`biquad`].

use serde::{Deserialize, Serialize};

use super::EffectContext;

mod biquad;

use biquad::{EqPointParams, HighEdgeParams, LowEdgeParams, MultibandEqState};

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
#[derive(Clone, Default, Serialize, Deserialize)]
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

impl super::core::DspEffect for MultibandEqEffect {
    fn process(&mut self, samples: &[f32], context: &EffectContext, _drain: bool) -> Vec<f32> {
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

    fn reset_state(&mut self) {
        if let Some(state) = self.state.as_mut() {
            state.reset();
        }
        self.state = None;
    }
}

impl MultibandEqEffect {
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

#[cfg(test)]
mod tests {
    use super::super::core::DspEffect;
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
