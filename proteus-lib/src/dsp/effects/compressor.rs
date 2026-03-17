//! Compressor effect for dynamic range control.

use serde::{Deserialize, Serialize};

use super::core::level::deserialize_db_gain;
use super::EffectContext;
use crate::dsp::guardrails::{
    sanitize_channels, sanitize_finite, sanitize_finite_max, sanitize_finite_min,
};

const DEFAULT_THRESHOLD_DB: f32 = -18.0;
const DEFAULT_RATIO: f32 = 4.0;
const DEFAULT_ATTACK_MS: f32 = 10.0;
const DEFAULT_RELEASE_MS: f32 = 100.0;
const DEFAULT_MAKEUP_DB: f32 = 0.0;

/// Serialized configuration for compressor parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CompressorSettings {
    /// Signal level above which compression is applied, in dBFS.
    #[serde(
        alias = "threshold",
        alias = "threshold_db",
        deserialize_with = "deserialize_db_gain"
    )]
    pub threshold_db: f32,
    /// Compression ratio; e.g. 4.0 means a 4:1 ratio above the threshold.
    pub ratio: f32,
    /// Time for the compressor gain to react to a signal exceeding the threshold, in milliseconds.
    #[serde(alias = "attack_ms", alias = "attack")]
    pub attack_ms: f32,
    /// Time for the compressor gain to recover after the signal drops below the threshold, in milliseconds.
    #[serde(alias = "release_ms", alias = "release")]
    pub release_ms: f32,
    /// Makeup gain applied after compression to restore loudness, in dB.
    #[serde(
        alias = "makeup_db",
        alias = "makeup_gain",
        alias = "makeup_gain_db",
        deserialize_with = "deserialize_db_gain"
    )]
    pub makeup_gain_db: f32,
}

impl CompressorSettings {
    /// Create compressor settings.
    pub fn new(
        threshold_db: f32,
        ratio: f32,
        attack_ms: f32,
        release_ms: f32,
        makeup_gain_db: f32,
    ) -> Self {
        Self {
            threshold_db,
            ratio,
            attack_ms,
            release_ms,
            makeup_gain_db,
        }
    }
}

impl Default for CompressorSettings {
    fn default() -> Self {
        Self {
            threshold_db: DEFAULT_THRESHOLD_DB,
            ratio: DEFAULT_RATIO,
            attack_ms: DEFAULT_ATTACK_MS,
            release_ms: DEFAULT_RELEASE_MS,
            makeup_gain_db: DEFAULT_MAKEUP_DB,
        }
    }
}

/// Configured compressor effect with runtime state.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CompressorEffect {
    /// Whether the compressor is active; when `false` samples pass through unmodified.
    pub enabled: bool,
    /// Compressor parameters such as threshold, ratio, attack, and release.
    #[serde(flatten)]
    pub settings: CompressorSettings,
    #[serde(skip)]
    state: Option<CompressorState>,
}

impl std::fmt::Debug for CompressorEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompressorEffect")
            .field("enabled", &self.enabled)
            .field("settings", &self.settings)
            .finish()
    }
}

impl super::core::DspEffect for CompressorEffect {
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

        for frame in samples.chunks(channels) {
            let mut peak = 0.0_f32;
            for &sample in frame {
                peak = peak.max(sample.abs());
            }

            let level_db = rodio::math::linear_to_db(peak);
            let target_gain_db = compute_gain_db(level_db, state.threshold_db, state.ratio);
            state.update_gain(target_gain_db);
            let gain = rodio::math::db_to_linear(state.current_gain_db + state.makeup_gain_db);

            for &sample in frame {
                output.push(sample * gain);
            }
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

impl CompressorEffect {
    fn ensure_state(&mut self, context: &EffectContext) {
        let threshold_db =
            sanitize_finite_max(self.settings.threshold_db, DEFAULT_THRESHOLD_DB, 0.0);
        let ratio = sanitize_finite_min(self.settings.ratio, DEFAULT_RATIO, 1.0);
        let attack_ms = sanitize_finite_min(self.settings.attack_ms, DEFAULT_ATTACK_MS, 0.0);
        let release_ms = sanitize_finite_min(self.settings.release_ms, DEFAULT_RELEASE_MS, 0.0);
        let makeup_gain_db = sanitize_finite(self.settings.makeup_gain_db, DEFAULT_MAKEUP_DB);
        let channels = sanitize_channels(context.channels);

        let params = CompressorParams {
            sample_rate: context.sample_rate,
            channels,
            threshold_db,
            ratio,
            attack_ms,
            release_ms,
            makeup_gain_db,
        };
        let needs_reset = self
            .state
            .as_ref()
            .map(|state| !state.matches(&params))
            .unwrap_or(true);

        if needs_reset {
            self.state = Some(CompressorState::new(&params));
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct CompressorParams {
    sample_rate: u32,
    channels: usize,
    threshold_db: f32,
    ratio: f32,
    attack_ms: f32,
    release_ms: f32,
    makeup_gain_db: f32,
}

#[derive(Clone, Debug)]
struct CompressorState {
    sample_rate: u32,
    channels: usize,
    threshold_db: f32,
    ratio: f32,
    attack_coeff: f32,
    release_coeff: f32,
    makeup_gain_db: f32,
    current_gain_db: f32,
}

impl CompressorState {
    fn new(params: &CompressorParams) -> Self {
        let attack_coeff = time_to_coeff(params.attack_ms, params.sample_rate);
        let release_coeff = time_to_coeff(params.release_ms, params.sample_rate);
        Self {
            sample_rate: params.sample_rate,
            channels: params.channels,
            threshold_db: params.threshold_db,
            ratio: params.ratio,
            attack_coeff,
            release_coeff,
            makeup_gain_db: params.makeup_gain_db,
            current_gain_db: 0.0,
        }
    }

    fn matches(&self, params: &CompressorParams) -> bool {
        self.sample_rate == params.sample_rate
            && self.channels == params.channels
            && (self.threshold_db - params.threshold_db).abs() < f32::EPSILON
            && (self.ratio - params.ratio).abs() < f32::EPSILON
            && (self.attack_coeff - time_to_coeff(params.attack_ms, params.sample_rate)).abs()
                < f32::EPSILON
            && (self.release_coeff - time_to_coeff(params.release_ms, params.sample_rate)).abs()
                < f32::EPSILON
            && (self.makeup_gain_db - params.makeup_gain_db).abs() < f32::EPSILON
    }

    fn update_gain(&mut self, target_gain_db: f32) {
        let coeff = if target_gain_db < self.current_gain_db {
            self.attack_coeff
        } else {
            self.release_coeff
        };
        self.current_gain_db = coeff * self.current_gain_db + (1.0 - coeff) * target_gain_db;
    }

    fn reset(&mut self) {
        self.current_gain_db = 0.0;
    }
}

fn compute_gain_db(level_db: f32, threshold_db: f32, ratio: f32) -> f32 {
    if level_db <= threshold_db {
        0.0
    } else {
        let compressed = threshold_db + (level_db - threshold_db) / ratio;
        compressed - level_db
    }
}

fn time_to_coeff(time_ms: f32, sample_rate: u32) -> f32 {
    if time_ms <= 0.0 || !time_ms.is_finite() {
        return 0.0;
    }
    let t = time_ms / 1000.0;
    (-1.0 / (t * sample_rate as f32)).exp()
}

#[cfg(test)]
mod tests {
    use super::CompressorEffect;
    use crate::dsp::effects::{core::DspEffect, EffectContext};

    fn context(channels: usize) -> EffectContext {
        EffectContext {
            sample_rate: 48_000,
            channels,
            container_path: None,
            impulse_response_spec: None,
            impulse_response_tail_db: -60.0,
        }
    }

    fn approx_eq(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn compressor_disabled_passthrough() {
        let mut effect = CompressorEffect::default();
        let samples = vec![0.25_f32, -0.25, 0.5, -0.5];
        let output = effect.process(&samples, &context(2), false);
        assert_eq!(output, samples);
    }

    #[test]
    fn compressor_applies_gain_reduction() {
        let mut effect = CompressorEffect::default();
        effect.enabled = true;
        effect.settings.threshold_db = -6.0;
        effect.settings.ratio = 2.0;
        effect.settings.attack_ms = 0.0;
        effect.settings.release_ms = 0.0;
        effect.settings.makeup_gain_db = 0.0;

        let samples = vec![1.0_f32, 1.0];
        let output = effect.process(&samples, &context(2), false);
        assert_eq!(output.len(), samples.len());

        let level_db = 0.0;
        let target_gain_db = (-6.0 + (level_db + 6.0) / 2.0) - level_db;
        let expected = rodio::math::db_to_linear(target_gain_db);
        assert!(output.iter().all(|value| approx_eq(*value, expected, 1e-3)));
    }

    #[test]
    fn compressor_deserializes_db_and_linear_strings() {
        let json = r#"{
            "enabled": true,
            "threshold_db": "-12db",
            "ratio": 2.0,
            "attack_ms": 5.0,
            "release_ms": 50.0,
            "makeup_gain_db": "2.0"
        }"#;

        let effect: CompressorEffect = serde_json::from_str(json).expect("deserialize compressor");
        assert!(approx_eq(effect.settings.threshold_db, -12.0, 1e-6));
        assert!(approx_eq(
            effect.settings.makeup_gain_db,
            rodio::math::linear_to_db(2.0),
            1e-6
        ));
    }

    #[test]
    fn compressor_rejects_non_positive_linear_string_for_db_fields() {
        let json = r#"{
            "enabled": true,
            "threshold_db": "-2",
            "ratio": 2.0,
            "attack_ms": 5.0,
            "release_ms": 50.0,
            "makeup_gain_db": "0"
        }"#;

        let err = serde_json::from_str::<CompressorEffect>(json).expect_err("invalid compressor");
        assert!(err.to_string().contains("invalid gain value"));
    }
}
