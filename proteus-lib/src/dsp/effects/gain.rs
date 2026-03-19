//! Simple gain effect.

use serde::{Deserialize, Serialize};

use super::core::level::deserialize_linear_gain;
use super::core::smoother::ParamSmoother;
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
    #[serde(skip)]
    smoother: Option<ParamSmoother>,
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
    fn process(&mut self, samples: &[f32], context: &EffectContext, _drain: bool) -> Vec<f32> {
        if !self.enabled {
            return samples.to_vec();
        }
        if samples.is_empty() {
            return Vec::new();
        }

        let target = sanitize_finite(self.settings.gain, DEFAULT_GAIN);
        let smoother = self.ensure_smoother(target, context);
        let channels = context.channels().max(1);

        let mut out = Vec::with_capacity(samples.len());
        if smoother.is_settled() {
            let gain = smoother.current();
            for &sample in samples {
                out.push(sample * gain);
            }
        } else {
            for frame in samples.chunks(channels) {
                let gain = smoother.next();
                for &sample in frame {
                    out.push(sample * gain);
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

        let target = sanitize_finite(self.settings.gain, DEFAULT_GAIN);
        let smoother = self.ensure_smoother(target, context);
        let channels = context.channels().max(1);

        if smoother.is_settled() {
            let gain = smoother.current();
            for &sample in input {
                output.push(sample * gain);
            }
        } else {
            for frame in input.chunks(channels) {
                let gain = smoother.next();
                for &sample in frame {
                    output.push(sample * gain);
                }
            }
        }
    }

    fn reset_state(&mut self) {
        self.smoother = None;
    }
}

impl GainEffect {
    fn ensure_smoother(&mut self, target: f32, context: &EffectContext) -> &mut ParamSmoother {
        let smoother = self
            .smoother
            .get_or_insert_with(|| ParamSmoother::new(target));
        if (smoother.target() - target).abs() > f32::EPSILON {
            smoother.set_target(target, context.parameter_ramp_samples());
        }
        smoother
    }
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

    #[test]
    fn gain_sweep_stays_continuous_on_sine_wave() {
        let mut effect = GainEffect::default();
        effect.enabled = true;
        effect.settings.gain = 0.8;

        let mut context = EffectContext::new(48_000, 1, None, None, -60.0).unwrap();
        context.set_parameter_ramp_ms(5.0);

        let signal = (0..444)
            .map(|index| {
                let phase = 2.0 * std::f32::consts::PI * 1_000.0 * index as f32 / 48_000.0;
                phase.sin()
            })
            .collect::<Vec<_>>();

        let first = effect.process(&signal[..204], &context, false);
        effect.settings.gain = 1.2;
        let second = effect.process(&signal[204..], &context, false);

        let mut combined = first;
        combined.extend(second);

        let largest_delta = combined
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0_f32, f32::max);
        assert!(largest_delta < 0.25);
    }
}
