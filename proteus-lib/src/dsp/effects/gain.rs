//! Simple gain effect.

use serde::{Deserialize, Serialize};

use super::level::deserialize_linear_gain;
use super::EffectContext;

const DEFAULT_GAIN: f32 = 1.0;

/// Serialized configuration for gain parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GainSettings {
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
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GainEffect {
    pub enabled: bool,
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

impl Default for GainEffect {
    fn default() -> Self {
        Self {
            enabled: false,
            settings: GainSettings::default(),
        }
    }
}

impl GainEffect {
    /// Process interleaved samples through the gain effect.
    ///
    /// # Arguments
    /// - `samples`: Interleaved input samples.
    /// - `context`: Environment details (unused for this effect).
    /// - `drain`: Unused for this effect.
    ///
    /// # Returns
    /// Processed interleaved samples.
    pub fn process(&mut self, samples: &[f32], _context: &EffectContext, _drain: bool) -> Vec<f32> {
        if !self.enabled {
            return samples.to_vec();
        }

        let gain = sanitize_gain(self.settings.gain);
        if samples.is_empty() {
            return Vec::new();
        }

        let mut out = Vec::with_capacity(samples.len());
        for &sample in samples {
            out.push(sample * gain);
        }

        out
    }

    /// Reset any internal state (none for gain).
    pub fn reset_state(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::level::db_to_linear;

    fn context() -> EffectContext {
        EffectContext {
            sample_rate: 44_100,
            channels: 1,
            container_path: None,
            impulse_response_spec: None,
            impulse_response_tail_db: -60.0,
        }
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

fn sanitize_gain(gain: f32) -> f32 {
    if gain.is_finite() {
        gain
    } else {
        DEFAULT_GAIN
    }
}
