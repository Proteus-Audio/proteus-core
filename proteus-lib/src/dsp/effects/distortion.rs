//! Distortion effect based on rodio's distortion source.

use serde::{Deserialize, Serialize};

use super::core::level::deserialize_linear_gain;
use super::EffectContext;

const DEFAULT_GAIN: f32 = 1.0;
const DEFAULT_THRESHOLD: f32 = 1.0;

/// Serialized configuration for distortion parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DistortionSettings {
    /// Pre-distortion gain multiplier applied before hard clipping.
    #[serde(deserialize_with = "deserialize_linear_gain")]
    pub gain: f32,
    /// Clipping threshold; samples with absolute value above this are hard-clipped.
    #[serde(deserialize_with = "deserialize_linear_gain")]
    pub threshold: f32,
}

impl DistortionSettings {
    /// Create a distortion settings payload.
    pub fn new(gain: f32, threshold: f32) -> Self {
        Self { gain, threshold }
    }
}

impl Default for DistortionSettings {
    fn default() -> Self {
        Self {
            gain: DEFAULT_GAIN,
            threshold: DEFAULT_THRESHOLD,
        }
    }
}

/// Configured distortion effect.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DistortionEffect {
    /// Whether the distortion is active; when `false` samples pass through unmodified.
    pub enabled: bool,
    /// Distortion parameters such as pre-gain and clipping threshold.
    #[serde(flatten)]
    pub settings: DistortionSettings,
}

impl std::fmt::Debug for DistortionEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DistortionEffect")
            .field("enabled", &self.enabled)
            .field("settings", &self.settings)
            .finish()
    }
}

impl super::core::DspEffect for DistortionEffect {
    fn process(&mut self, samples: &[f32], _context: &EffectContext, _drain: bool) -> Vec<f32> {
        if !self.enabled {
            return samples.to_vec();
        }

        let gain = sanitize_gain(self.settings.gain);
        let threshold = sanitize_threshold(self.settings.threshold);
        if samples.is_empty() {
            return Vec::new();
        }

        let mut out = Vec::with_capacity(samples.len());
        for &sample in samples {
            let v = sample * gain;
            out.push(v.clamp(-threshold, threshold));
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
        let gain = sanitize_gain(self.settings.gain);
        let threshold = sanitize_threshold(self.settings.threshold);
        for &sample in input {
            output.push((sample * gain).clamp(-threshold, threshold));
        }
    }

    fn reset_state(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::super::core::DspEffect;
    use super::*;

    fn context() -> EffectContext {
        EffectContext::new(44_100, 1, None, None, -60.0).unwrap()
    }

    #[test]
    fn distortion_disabled_passthrough() {
        let mut effect = DistortionEffect::default();
        let samples = vec![0.25_f32, -0.25, 0.5, -0.5];
        let output = effect.process(&samples, &context(), false);
        assert_eq!(output, samples);
    }

    #[test]
    fn distortion_clamps_output() {
        let mut effect = DistortionEffect::default();
        effect.enabled = true;
        effect.settings.gain = 2.0;
        effect.settings.threshold = 0.5;
        let samples = vec![0.4_f32, -0.4, 0.6, -0.6];
        let output = effect.process(&samples, &context(), false);
        assert_eq!(output.len(), samples.len());
        assert_eq!(output[0], 0.5);
        assert_eq!(output[1], -0.5);
        assert_eq!(output[2], 0.5);
        assert_eq!(output[3], -0.5);
    }

    #[test]
    fn distortion_deserializes_db_gain() {
        let json = r#"{"enabled":true,"gain":"6db","threshold":"-6db"}"#;
        let effect: DistortionEffect = serde_json::from_str(json).expect("deserialize distortion");
        assert!(effect.settings.gain > 1.0);
        assert!(effect.settings.threshold > 0.0);
    }
}

fn sanitize_gain(gain: f32) -> f32 {
    if gain.is_finite() {
        gain
    } else {
        DEFAULT_GAIN
    }
}

fn sanitize_threshold(threshold: f32) -> f32 {
    if !threshold.is_finite() {
        return DEFAULT_THRESHOLD;
    }
    let t = threshold.abs();
    if t <= f32::EPSILON {
        DEFAULT_THRESHOLD
    } else {
        t
    }
}
