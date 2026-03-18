//! Distortion effect based on rodio's distortion source.

use serde::{Deserialize, Serialize};

use super::core::level::deserialize_linear_gain;
use super::core::smoother::ParamSmoother;
use super::EffectContext;
use crate::dsp::guardrails::sanitize_finite;

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
    #[serde(skip)]
    gain_smoother: Option<ParamSmoother>,
    #[serde(skip)]
    threshold_smoother: Option<ParamSmoother>,
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
    fn process(&mut self, samples: &[f32], context: &EffectContext, _drain: bool) -> Vec<f32> {
        if !self.enabled {
            return samples.to_vec();
        }
        if samples.is_empty() {
            return Vec::new();
        }

        self.update_smoothers(context);
        let gs = self.gain_smoother.as_mut().unwrap();
        let ts = self.threshold_smoother.as_mut().unwrap();
        let channels = context.channels().max(1);

        let mut out = Vec::with_capacity(samples.len());
        if gs.is_settled() && ts.is_settled() {
            let gain = gs.current();
            let threshold = ts.current();
            for &sample in samples {
                out.push((sample * gain).clamp(-threshold, threshold));
            }
        } else {
            for frame in samples.chunks(channels) {
                let gain = gs.next();
                let threshold = ts.next();
                for &sample in frame {
                    out.push((sample * gain).clamp(-threshold, threshold));
                }
            }
        }
        out
    }

    fn process_into(
        &mut self,
        input: &[f32],
        output: &mut Vec<f32>,
        context: &EffectContext,
        _drain: bool,
    ) {
        if !self.enabled {
            output.extend_from_slice(input);
            return;
        }

        self.update_smoothers(context);
        let gs = self.gain_smoother.as_mut().unwrap();
        let ts = self.threshold_smoother.as_mut().unwrap();
        let channels = context.channels().max(1);

        if gs.is_settled() && ts.is_settled() {
            let gain = gs.current();
            let threshold = ts.current();
            for &sample in input {
                output.push((sample * gain).clamp(-threshold, threshold));
            }
        } else {
            for frame in input.chunks(channels) {
                let gain = gs.next();
                let threshold = ts.next();
                for &sample in frame {
                    output.push((sample * gain).clamp(-threshold, threshold));
                }
            }
        }
    }

    fn reset_state(&mut self) {
        self.gain_smoother = None;
        self.threshold_smoother = None;
    }
}

impl DistortionEffect {
    fn update_smoothers(&mut self, context: &EffectContext) {
        let target_gain = sanitize_finite(self.settings.gain, DEFAULT_GAIN);
        let target_threshold = sanitize_threshold(self.settings.threshold);
        let ramp = context.parameter_ramp_samples();

        let gs = self
            .gain_smoother
            .get_or_insert_with(|| ParamSmoother::new(target_gain));
        if (gs.target() - target_gain).abs() > f32::EPSILON {
            gs.set_target(target_gain, ramp);
        }

        let ts = self
            .threshold_smoother
            .get_or_insert_with(|| ParamSmoother::new(target_threshold));
        if (ts.target() - target_threshold).abs() > f32::EPSILON {
            ts.set_target(target_threshold, ramp);
        }
    }
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

fn sanitize_threshold(threshold: f32) -> f32 {
    let value = sanitize_finite(threshold, DEFAULT_THRESHOLD);
    let t = value.abs();
    if t <= f32::EPSILON {
        DEFAULT_THRESHOLD
    } else {
        t
    }
}
