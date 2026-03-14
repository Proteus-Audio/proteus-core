//! Delay reverb effect using a simple feedback delay line.

use log::info;
use serde::{Deserialize, Serialize};

use super::EffectContext;

const DEFAULT_DURATION_MS: u64 = 100;
const MAX_AMPLITUDE: f32 = 0.8;

/// Serializable settings for the legacy delay-based reverb effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DelayReverbSettings {
    /// Length of the feedback delay line in milliseconds.
    pub duration_ms: u64,
    /// Feedback amplitude (gain applied on each echo); clamped to [0.0, 0.8].
    pub amplitude: f32,
}

impl DelayReverbSettings {
    /// Create delay reverb settings.
    pub fn new(duration_ms: u64, amplitude: f32) -> Self {
        Self {
            duration_ms: duration_ms.clamp(0, u64::MAX),
            amplitude: amplitude.clamp(0.0, MAX_AMPLITUDE),
        }
    }

    fn amplitude(&self) -> f32 {
        self.amplitude.clamp(0.0, MAX_AMPLITUDE)
    }
}

impl Default for DelayReverbSettings {
    fn default() -> Self {
        Self {
            duration_ms: DEFAULT_DURATION_MS,
            amplitude: 0.7,
        }
    }
}

/// Delay reverb effect (feedback delay + mix).
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DelayReverbEffect {
    /// Whether the effect is active; when `false` samples pass through unmodified.
    pub enabled: bool,
    /// Dry/wet mix ratio (0.0 = fully dry, 1.0 = fully wet).
    #[serde(alias = "dry_wet", alias = "wet_dry")]
    pub mix: f32,
    /// Delay reverb parameters such as delay duration and feedback amplitude.
    #[serde(flatten)]
    pub settings: DelayReverbSettings,
    #[serde(skip)]
    state: Option<DelayReverbState>,
}

impl Default for DelayReverbEffect {
    fn default() -> Self {
        Self {
            enabled: true,
            mix: 0.0,
            settings: DelayReverbSettings::default(),
            state: None,
        }
    }
}

impl std::fmt::Debug for DelayReverbEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DelayReverbEffect")
            .field("enabled", &self.enabled)
            .field("mix", &self.mix)
            .field("settings", &self.settings)
            .finish()
    }
}

impl crate::dsp::effects::core::DspEffect for DelayReverbEffect {
    fn process(&mut self, samples: &[f32], context: &EffectContext, drain: bool) -> Vec<f32> {
        self.ensure_state(context);
        if !self.enabled || self.mix <= 0.0 {
            return samples.to_vec();
        }

        let Some(state) = self.state.as_mut() else {
            return samples.to_vec();
        };

        let amplitude = if self.mix > 0.0 {
            self.mix.clamp(0.0, MAX_AMPLITUDE)
        } else {
            self.settings.amplitude()
        };

        if samples.is_empty() {
            if drain {
                return state.drain_tail(amplitude);
            }
            return Vec::new();
        }

        let mut output = Vec::with_capacity(samples.len());
        state.process_samples(samples, amplitude, &mut output);
        output
    }

    fn process_into(
        &mut self,
        input: &[f32],
        output: &mut Vec<f32>,
        context: &EffectContext,
        drain: bool,
    ) {
        self.ensure_state(context);
        if !self.enabled || self.mix <= 0.0 {
            output.extend_from_slice(input);
            return;
        }
        let Some(state) = self.state.as_mut() else {
            output.extend_from_slice(input);
            return;
        };
        let amplitude = if self.mix > 0.0 {
            self.mix.clamp(0.0, MAX_AMPLITUDE)
        } else {
            self.settings.amplitude()
        };
        if input.is_empty() {
            if drain {
                let tail = state.drain_tail(amplitude);
                output.extend(tail);
            }
            return;
        }
        state.process_samples(input, amplitude, output);
    }

    fn reset_state(&mut self) {
        if let Some(state) = self.state.as_mut() {
            state.reset();
        }
        self.state = None;
    }
}

impl DelayReverbEffect {
    /// Create a new delay reverb effect.
    pub fn new(mix: f32) -> Self {
        Self {
            mix: mix.clamp(0.0, 1.0),
            ..Default::default()
        }
    }

    /// Mutable access to settings.
    pub fn settings_mut(&mut self) -> &mut DelayReverbSettings {
        &mut self.settings
    }

    fn ensure_state(&mut self, context: &EffectContext) {
        let delay_samples = delay_samples(
            context.sample_rate,
            context.channels,
            self.settings.duration_ms,
        );
        let needs_reset = self
            .state
            .as_ref()
            .map(|state| state.delay_samples != delay_samples)
            .unwrap_or(true);
        if needs_reset {
            self.state = Some(DelayReverbState::new(delay_samples));
        }
    }
}

#[derive(Clone)]
struct DelayReverbState {
    delay_samples: usize,
    delay_line: Vec<f32>,
    write_pos: usize,
}

impl DelayReverbState {
    fn new(delay_samples: usize) -> Self {
        info!("using delay reverb");
        Self {
            delay_samples,
            delay_line: vec![0.0; delay_samples.max(1)],
            write_pos: 0,
        }
    }

    fn reset(&mut self) {
        self.delay_line.fill(0.0);
        self.write_pos = 0;
    }

    fn process_samples(&mut self, samples: &[f32], amplitude: f32, out: &mut Vec<f32>) {
        if self.delay_samples == 0 {
            out.extend_from_slice(samples);
            return;
        }

        let delay_len = self.delay_line.len();
        for &sample in samples {
            let delayed = self.delay_line[self.write_pos];
            let output = sample + (delayed * amplitude);
            out.push(output);

            // Feedback delay for smoother tails.
            self.delay_line[self.write_pos] = sample + (delayed * amplitude);
            self.write_pos += 1;
            if self.write_pos >= delay_len {
                self.write_pos = 0;
            }
        }
    }

    fn drain_tail(&mut self, amplitude: f32) -> Vec<f32> {
        if self.delay_samples == 0 {
            return Vec::new();
        }

        let delay_len = self.delay_line.len();
        let mut out = Vec::with_capacity(delay_len);
        for _ in 0..delay_len {
            let delayed = self.delay_line[self.write_pos];
            let output = delayed * amplitude;
            out.push(output);

            // Feed silence to decay the tail.
            self.delay_line[self.write_pos] = delayed * amplitude;
            self.write_pos += 1;
            if self.write_pos >= delay_len {
                self.write_pos = 0;
            }
        }

        out
    }
}

fn delay_samples(sample_rate: u32, channels: usize, duration_ms: u64) -> usize {
    if duration_ms == 0 {
        return 0;
    }
    let ns = duration_ms.saturating_mul(1_000_000);
    let samples = ns.saturating_mul(sample_rate as u64) / 1_000_000_000 * channels as u64;
    samples as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::effects::core::DspEffect;

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
    fn delay_samples_handles_zero_duration() {
        assert_eq!(delay_samples(48_000, 2, 0), 0);
    }

    #[test]
    fn delay_reverb_passthrough_when_mix_is_zero() {
        let mut effect = DelayReverbEffect::new(0.0);
        effect.enabled = true;
        let input = vec![0.2_f32, -0.2, 0.3, -0.3];
        let output = effect.process(&input, &context(), false);
        assert_eq!(output, input);
    }

    #[test]
    fn delay_reverb_process_preserves_length() {
        let mut effect = DelayReverbEffect::new(0.4);
        effect.settings.duration_ms = 20;
        let input = vec![0.5_f32, -0.5, 0.25, -0.25];
        let output = effect.process(&input, &context(), false);
        assert_eq!(output.len(), input.len());
    }
}
