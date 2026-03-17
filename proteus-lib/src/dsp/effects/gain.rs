//! Simple gain effect.

use serde::{Deserialize, Serialize};

use super::core::level::deserialize_linear_gain;
use super::EffectContext;
use crate::dsp::guardrails::sanitize_finite;

const DEFAULT_GAIN: f32 = 1.0;

/// Serialized configuration for gain parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GainSettings {
    /// Linear amplitude multiplier applied to every sample (1.0 = unity gain).
    #[serde(deserialize_with = "deserialize_linear_gain")]
    pub gain: f32,
}

impl GainSettings {
    /// Create a gain settings payload.
    pub fn new(gain: f32) -> Self {
        Self { gain }
    }
}

impl Default for GainSettings {
    fn default() -> Self {
        Self { gain: DEFAULT_GAIN }
    }
}

/// Configured gain effect.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GainEffect {
    /// Whether the gain effect is active; when `false` samples pass through unmodified.
    pub enabled: bool,
    /// Gain parameter (linear multiplier).
    #[serde(flatten)]
    pub settings: GainSettings,
}

impl std::fmt::Debug for GainEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GainEffect")
            .field("enabled", &self.enabled)
            .field("settings", &self.settings)
            .finish()
    }
}

impl super::core::DspEffect for GainEffect {
    fn process(&mut self, samples: &[f32], _context: &EffectContext, _drain: bool) -> Vec<f32> {
        if !self.enabled {
            return samples.to_vec();
        }

        let gain = sanitize_finite(self.settings.gain, DEFAULT_GAIN);
        if samples.is_empty() {
            return Vec::new();
        }

        let mut out = Vec::with_capacity(samples.len());
        for &sample in samples {
            out.push(sample * gain);
        }

        out
    }

    fn process_into(
        &mut self,
        input: &[f32],
        output: &mut Vec<f32>,
        _context: &EffectContext,
        _drain: bool,
    ) {
        if !self.enabled {
            output.extend_from_slice(input);
            return;
        }
        let gain = sanitize_finite(self.settings.gain, DEFAULT_GAIN);
        for &sample in input {
            output.push(sample * gain);
        }
    }

    fn reset_state(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::super::core::DspEffect;
    use super::*;
    use rodio::math::db_to_linear;

    fn context() -> EffectContext {
        EffectContext::new(44_100, 1, None, None, -60.0).unwrap()
    }

    #[test]
    fn gain_disabled_passthrough() {
        let mut effect = GainEffect::default();
        let samples = vec![0.25_f32, -0.25, 0.5, -0.5];
        let output = effect.process(&samples, &context(), false);
        assert_eq!(output, samples);
    }

    #[test]
    fn gain_scales_samples() {
        let mut effect = GainEffect::default();
        effect.enabled = true;
        effect.settings.gain = 2.0;
        let samples = vec![0.25_f32, -0.25, 0.5, -0.5];
        let output = effect.process(&samples, &context(), false);
        assert_eq!(output, vec![0.5_f32, -0.5, 1.0, -1.0]);
    }

    #[test]
    fn gain_deserializes_db_strings() {
        let json = r#"{"enabled":true,"gain":"6db"}"#;
        let effect: GainEffect = serde_json::from_str(json).expect("deserialize gain");
        let expected = db_to_linear(6.0);
        assert!((effect.settings.gain - expected).abs() < 1e-6);
    }

    #[test]
    fn gain_deserializes_negative_db_strings() {
        let json = r#"{"enabled":true,"gain":"-2db"}"#;
        let effect: GainEffect = serde_json::from_str(json).expect("deserialize gain");
        let expected = db_to_linear(-2.0);
        assert!((effect.settings.gain - expected).abs() < 1e-6);
    }
}
