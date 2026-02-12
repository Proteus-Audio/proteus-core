//! Distortion effect based on rodio's distortion source.

use serde::{Deserialize, Serialize};

use super::level::deserialize_linear_gain;
use super::EffectContext;

const DEFAULT_GAIN: f32 = 1.0;
const DEFAULT_THRESHOLD: f32 = 1.0;

/// Serialized configuration for distortion parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DistortionSettings {
    #[serde(deserialize_with = "deserialize_linear_gain")]
    pub gain: f32,
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
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DistortionEffect {
    pub enabled: bool,
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

impl Default for DistortionEffect {
    fn default() -> Self {
        Self {
            enabled: false,
            settings: DistortionSettings::default(),
        }
    }
}

impl DistortionEffect {
    /// Process interleaved samples through the distortion effect.
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

    /// Reset any internal state (none for distortion).
    pub fn reset_state(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

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
