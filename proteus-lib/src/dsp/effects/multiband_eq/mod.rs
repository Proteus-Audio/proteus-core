//! Multiband parametric EQ effect.
//!
//! This effect applies a configurable chain of parametric EQ points plus
//! optional edge-shaping filters for low and high frequency boundaries.
//!
//! Biquad filter primitives and coefficient computation live in the private
//! `biquad` module.

use serde::{Deserialize, Serialize};

use super::EffectContext;
use crate::dsp::guardrails::{sanitize_channels, sanitize_finite_clamped, sanitize_freq};

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
    /// Center frequency of the parametric band, in Hz.
    pub freq_hz: u32,
    /// Quality factor controlling the bandwidth of the parametric band.
    pub q: f32,
    /// Boost or cut applied at the center frequency, in decibels.
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
    /// High-pass filter that attenuates frequencies below `freq_hz`.
    HighPass {
        /// Cutoff frequency in Hz.
        freq_hz: u32,
        /// Quality factor controlling the steepness of the rolloff.
        q: f32,
    },
    /// Low-shelf filter that boosts or cuts energy below `freq_hz`.
    LowShelf {
        /// Shelf center frequency in Hz.
        freq_hz: u32,
        /// Quality factor controlling the transition slope.
        q: f32,
        /// Gain applied in the shelf region, in decibels.
        gain_db: f32,
    },
}

/// Optional high-edge shaping.
///
/// `LowPass` removes high-end energy above the cutoff.
/// `HighShelf` boosts/cuts the high-end around the center frequency.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HighEdgeFilterSettings {
    /// Low-pass filter that attenuates frequencies above `freq_hz`.
    LowPass {
        /// Cutoff frequency in Hz.
        freq_hz: u32,
        /// Quality factor controlling the steepness of the rolloff.
        q: f32,
    },
    /// High-shelf filter that boosts or cuts energy above `freq_hz`.
    HighShelf {
        /// Shelf center frequency in Hz.
        freq_hz: u32,
        /// Quality factor controlling the transition slope.
        q: f32,
        /// Gain applied in the shelf region, in decibels.
        gain_db: f32,
    },
}

/// Serialized configuration for multiband EQ.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MultibandEqSettings {
    /// Parametric EQ bands applied in order from low to high frequency.
    #[serde(alias = "bands", alias = "eq_points")]
    pub points: Vec<EqPointSettings>,
    /// Optional low-edge filter (high-pass or low-shelf) applied before the parametric bands.
    pub low_edge: Option<LowEdgeFilterSettings>,
    /// Optional high-edge filter (low-pass or high-shelf) applied after the parametric bands.
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
    /// Whether the EQ effect is active; when `false` samples pass through unmodified.
    pub enabled: bool,
    /// Parametric EQ configuration including bands and edge filters.
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
        let channels = sanitize_channels(context.channels);
        let points = self
            .settings
            .points
            .iter()
            .map(|point| EqPointParams {
                freq_hz: sanitize_freq(point.freq_hz, context.sample_rate),
                q: sanitize_finite_clamped(point.q, DEFAULT_Q, MIN_Q, MAX_Q),
                gain_db: sanitize_finite_clamped(
                    point.gain_db,
                    DEFAULT_GAIN_DB,
                    MIN_GAIN_DB,
                    MAX_GAIN_DB,
                ),
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
            q: sanitize_finite_clamped(*q, DEFAULT_Q, MIN_Q, MAX_Q),
        },
        LowEdgeFilterSettings::LowShelf {
            freq_hz,
            q,
            gain_db,
        } => LowEdgeParams::LowShelf {
            freq_hz: sanitize_freq(*freq_hz, sample_rate),
            q: sanitize_finite_clamped(*q, DEFAULT_Q, MIN_Q, MAX_Q),
            gain_db: sanitize_finite_clamped(*gain_db, DEFAULT_GAIN_DB, MIN_GAIN_DB, MAX_GAIN_DB),
        },
    }
}

fn sanitize_high_edge(edge: &HighEdgeFilterSettings, sample_rate: u32) -> HighEdgeParams {
    match edge {
        HighEdgeFilterSettings::LowPass { freq_hz, q } => HighEdgeParams::LowPass {
            freq_hz: sanitize_freq(*freq_hz, sample_rate),
            q: sanitize_finite_clamped(*q, DEFAULT_Q, MIN_Q, MAX_Q),
        },
        HighEdgeFilterSettings::HighShelf {
            freq_hz,
            q,
            gain_db,
        } => HighEdgeParams::HighShelf {
            freq_hz: sanitize_freq(*freq_hz, sample_rate),
            q: sanitize_finite_clamped(*q, DEFAULT_Q, MIN_Q, MAX_Q),
            gain_db: sanitize_finite_clamped(*gain_db, DEFAULT_GAIN_DB, MIN_GAIN_DB, MAX_GAIN_DB),
        },
    }
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
