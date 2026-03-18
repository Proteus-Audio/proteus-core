//! Low-pass filter effect based on rodio's BLT filter.

use serde::{Deserialize, Serialize};

use super::core::biquad::{BiquadKind, BiquadState};
use super::EffectContext;

const DEFAULT_FREQ_HZ: u32 = 1000;
const DEFAULT_Q: f32 = 0.5;

/// Serialized configuration for low-pass filter parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LowPassFilterSettings {
    /// Cutoff frequency in Hz; energy above this frequency is attenuated.
    #[serde(alias = "freq", alias = "frequency_hz")]
    pub freq_hz: u32,
    /// Quality factor controlling the sharpness of the cutoff slope.
    #[serde(alias = "bandwidth")]
    pub q: f32,
}

impl LowPassFilterSettings {
    /// Create a low-pass settings payload.
    pub fn new(freq_hz: u32, q: f32) -> Self {
        Self { freq_hz, q }
    }
}

impl Default for LowPassFilterSettings {
    fn default() -> Self {
        Self {
            freq_hz: DEFAULT_FREQ_HZ,
            q: DEFAULT_Q,
        }
    }
}

/// Configured low-pass filter effect with runtime state.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LowPassFilterEffect {
    /// Whether the filter is active; when `false` samples pass through unmodified.
    pub enabled: bool,
    /// Low-pass filter parameters such as cutoff frequency and Q factor.
    #[serde(flatten)]
    pub settings: LowPassFilterSettings,
    #[serde(skip)]
    state: Option<BiquadState>,
}

impl std::fmt::Debug for LowPassFilterEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LowPassFilterEffect")
            .field("enabled", &self.enabled)
            .field("settings", &self.settings)
            .finish()
    }
}

impl super::core::DspEffect for LowPassFilterEffect {
    fn process(&mut self, samples: &[f32], context: &EffectContext, _drain: bool) -> Vec<f32> {
        if !self.enabled {
            return samples.to_vec();
        }

        self.ensure_state(context);
        let Some(state) = self.state.as_mut() else {
            return samples.to_vec();
        };

        state.process(samples)
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
        self.ensure_state(context);
        let Some(state) = self.state.as_mut() else {
            output.extend_from_slice(input);
            return;
        };
        state.process_into(input, output);
    }

    fn reset_state(&mut self) {
        if let Some(state) = self.state.as_mut() {
            state.reset();
        }
        self.state = None;
    }
}

impl LowPassFilterEffect {
    fn ensure_state(&mut self, context: &EffectContext) {
        if let Some(state) = self.state.as_mut() {
            if state.matches_structure(
                BiquadKind::LowPass,
                context.sample_rate(),
                context.channels(),
            ) {
                // Structure unchanged — smoothly ramp to new coefficients if
                // freq/Q have changed, preserving the delay line.
                state.update_coefficients(
                    self.settings.freq_hz,
                    self.settings.q,
                    context.parameter_ramp_samples(),
                );
                return;
            }
        }

        // Full reconstruction needed (first call, or sample_rate/channels changed).
        self.state = Some(BiquadState::new(
            BiquadKind::LowPass,
            context.sample_rate(),
            context.channels(),
            self.settings.freq_hz,
            self.settings.q,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::super::core::DspEffect;
    use super::*;

    fn context() -> EffectContext {
        EffectContext::new(48_000, 2, None, None, -60.0).unwrap()
    }

    #[test]
    fn low_pass_disabled_passthrough() {
        let mut effect = LowPassFilterEffect::default();
        let samples = vec![0.1_f32, -0.1, 0.2, -0.2];
        let output = effect.process(&samples, &context(), false);
        assert_eq!(output, samples);
    }

    #[test]
    fn low_pass_enabled_changes_signal() {
        let mut effect = LowPassFilterEffect::default();
        effect.enabled = true;
        effect.settings.freq_hz = 200;
        let samples = vec![1.0_f32, -1.0, 1.0, -1.0, 1.0, -1.0];
        let output = effect.process(&samples, &context(), false);
        assert_eq!(output.len(), samples.len());
        assert!(output.iter().any(|value| (*value - 1.0).abs() > 1e-6));
        assert!(output.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn low_pass_reset_restores_passthrough_when_disabled() {
        let mut effect = LowPassFilterEffect::default();
        effect.enabled = true;
        effect.settings.freq_hz = 350;
        let samples = vec![0.3_f32, -0.4, 0.9, -0.8];
        let _ = effect.process(&samples, &context(), false);

        effect.reset_state();
        effect.enabled = false;
        let output = effect.process(&samples, &context(), false);
        assert_eq!(output, samples);
    }

    #[test]
    fn low_pass_cutoff_change_stays_continuous() {
        let mut effect = LowPassFilterEffect::default();
        effect.enabled = true;
        effect.settings.freq_hz = 300;

        let mut context = context();
        context.set_parameter_ramp_ms(5.0);

        let signal = (0..512)
            .map(|index| {
                let phase = 2.0 * std::f32::consts::PI * 1_000.0 * index as f32 / 48_000.0;
                phase.sin()
            })
            .flat_map(|sample| [sample, sample])
            .collect::<Vec<_>>();

        let first = effect.process(&signal[..256], &context, false);
        let previous = *first.last().unwrap();
        effect.settings.freq_hz = 4_000;
        let second = effect.process(&signal[256..], &context, false);

        assert!((second[0] - previous).abs() < 0.2);
        assert!(second.iter().all(|sample| sample.is_finite()));
    }
}
