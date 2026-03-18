//! Stereo pan effect.

use serde::{Deserialize, Serialize};

use super::core::smoother::ParamSmoother;
use super::EffectContext;
use crate::dsp::guardrails::sanitize_finite_clamped;

const DEFAULT_PAN: f32 = 0.0;

/// Serialized configuration for pan parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PanSettings {
    /// Stereo pan position in the range `[-1.0, 1.0]`.
    ///
    /// - `-1.0`: hard left
    /// - `0.0`: center
    /// - `1.0`: hard right
    #[serde(alias = "position")]
    pub pan: f32,
}

impl PanSettings {
    /// Create a pan settings payload.
    pub fn new(pan: f32) -> Self {
        Self { pan }
    }
}

impl Default for PanSettings {
    fn default() -> Self {
        Self { pan: DEFAULT_PAN }
    }
}

/// Configured pan effect.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PanEffect {
    /// Whether the pan effect is active; when `false` samples pass through unmodified.
    pub enabled: bool,
    /// Pan parameter controlling the stereo position.
    #[serde(flatten)]
    pub settings: PanSettings,
    #[serde(skip)]
    smoother: Option<ParamSmoother>,
}

impl std::fmt::Debug for PanEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PanEffect")
            .field("enabled", &self.enabled)
            .field("settings", &self.settings)
            .finish()
    }
}

impl super::core::DspEffect for PanEffect {
    fn process(&mut self, samples: &[f32], context: &EffectContext, _drain: bool) -> Vec<f32> {
        if !self.enabled {
            return samples.to_vec();
        }
        if samples.is_empty() {
            return Vec::new();
        }
        if context.channels() != 2 {
            return samples.to_vec();
        }

        let target = sanitize_finite_clamped(self.settings.pan, DEFAULT_PAN, -1.0, 1.0);
        let smoother = self.ensure_smoother(target, context);

        let mut out = Vec::with_capacity(samples.len());
        if smoother.is_settled() {
            let (left_gain, right_gain) = pan_gains(smoother.current());
            let mut chunks = samples.chunks_exact(2);
            for frame in &mut chunks {
                out.push(frame[0] * left_gain);
                out.push(frame[1] * right_gain);
            }
            out.extend_from_slice(chunks.remainder());
        } else {
            let mut chunks = samples.chunks_exact(2);
            for frame in &mut chunks {
                let (lg, rg) = pan_gains(smoother.next());
                out.push(frame[0] * lg);
                out.push(frame[1] * rg);
            }
            out.extend_from_slice(chunks.remainder());
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
        if !self.enabled || input.is_empty() || context.channels() != 2 {
            output.extend_from_slice(input);
            return;
        }

        let target = sanitize_finite_clamped(self.settings.pan, DEFAULT_PAN, -1.0, 1.0);
        let smoother = self.ensure_smoother(target, context);

        if smoother.is_settled() {
            let (left_gain, right_gain) = pan_gains(smoother.current());
            let mut chunks = input.chunks_exact(2);
            for frame in &mut chunks {
                output.push(frame[0] * left_gain);
                output.push(frame[1] * right_gain);
            }
            output.extend_from_slice(chunks.remainder());
        } else {
            let mut chunks = input.chunks_exact(2);
            for frame in &mut chunks {
                let (lg, rg) = pan_gains(smoother.next());
                output.push(frame[0] * lg);
                output.push(frame[1] * rg);
            }
            output.extend_from_slice(chunks.remainder());
        }
    }

    fn reset_state(&mut self) {
        self.smoother = None;
    }
}

impl PanEffect {
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

fn pan_gains(pan: f32) -> (f32, f32) {
    let theta = ((pan + 1.0) * std::f32::consts::FRAC_PI_4).clamp(0.0, std::f32::consts::FRAC_PI_2);
    (theta.cos(), theta.sin())
}

#[cfg(test)]
mod tests {
    use super::super::core::DspEffect;
    use super::*;

    fn stereo_context() -> EffectContext {
        EffectContext::new(48_000, 2, None, None, -60.0).unwrap()
    }

    fn mono_context() -> EffectContext {
        EffectContext::new(48_000, 1, None, None, -60.0).unwrap()
    }

    #[test]
    fn pan_disabled_passthrough() {
        let mut effect = PanEffect::default();
        let samples = vec![0.25_f32, -0.25, 0.5, -0.5];
        let output = effect.process(&samples, &stereo_context(), false);
        assert_eq!(output, samples);
    }

    #[test]
    fn pan_hard_left_mutes_right_lane() {
        let mut effect = PanEffect::default();
        effect.enabled = true;
        effect.settings.pan = -1.0;
        let samples = vec![1.0_f32, 1.0, 0.5, 0.5];
        let output = effect.process(&samples, &stereo_context(), false);
        assert_eq!(output, vec![1.0_f32, 0.0, 0.5, 0.0]);
    }

    #[test]
    fn pan_hard_right_mutes_left_lane() {
        let mut effect = PanEffect::default();
        effect.enabled = true;
        effect.settings.pan = 1.0;
        let samples = vec![1.0_f32, 1.0, 0.5, 0.5];
        let output = effect.process(&samples, &stereo_context(), false);
        assert_eq!(output.len(), 4);
        assert!(output[0].abs() < 1e-6);
        assert!((output[1] - 1.0).abs() < 1e-6);
        assert!(output[2].abs() < 1e-6);
        assert!((output[3] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn pan_non_stereo_passthrough() {
        let mut effect = PanEffect::default();
        effect.enabled = true;
        effect.settings.pan = 0.75;
        let samples = vec![0.1_f32, -0.2, 0.3, -0.4];
        let output = effect.process(&samples, &mono_context(), false);
        assert_eq!(output, samples);
    }

    #[test]
    fn pan_changes_ramp_over_multiple_frames() {
        let mut effect = PanEffect::default();
        effect.enabled = true;

        let mut context = stereo_context();
        context.set_parameter_ramp_ms(0.5);

        let _ = effect.process(&[1.0_f32, 1.0, 1.0, 1.0], &context, false);
        effect.settings.pan = 1.0;
        let output = effect.process(&[1.0_f32, 1.0, 1.0, 1.0], &context, false);

        assert!(output[0] > 0.0);
        assert!(output[0] < 1.0);
        assert!(output[1] > 0.0);
        assert!(output[1] < 1.0);
    }
}
