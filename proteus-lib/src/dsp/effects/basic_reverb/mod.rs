//! Basic reverb effect powered by rodio's `reverb` source.

use std::collections::VecDeque;
use std::time::Duration;

use rodio::buffer::SamplesBuffer;
use rodio::Source;
use serde::{Deserialize, Serialize};

use super::EffectContext;

const DEFAULT_DURATION_MS: u64 = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BasicReverbSettings {
    pub duration_ms: u64,
    pub amplitude: f32,
}

impl Default for BasicReverbSettings {
    fn default() -> Self {
        Self {
            duration_ms: DEFAULT_DURATION_MS,
            amplitude: 0.7,
        }
    }
}

/// Basic rodio reverb effect (delay + mix).
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BasicReverbEffect {
    pub enabled: bool,
    pub mix: f32,
    #[serde(flatten)]
    pub settings: BasicReverbSettings,
    #[serde(skip)]
    state: Option<BasicReverbState>,
}

impl Default for BasicReverbEffect {
    fn default() -> Self {
        Self {
            enabled: true,
            mix: 0.0,
            settings: BasicReverbSettings::default(),
            state: None,
        }
    }
}

impl std::fmt::Debug for BasicReverbEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BasicReverbEffect")
            .field("enabled", &self.enabled)
            .field("mix", &self.mix)
            .field("settings", &self.settings)
            .finish()
    }
}

impl BasicReverbEffect {
    /// Create a new basic reverb effect.
    pub fn new(mix: f32) -> Self {
        Self {
            mix: mix.clamp(0.0, 1.0),
            ..Default::default()
        }
    }

    /// Process interleaved samples through rodio's reverb.
    pub fn process(&mut self, samples: &[f32], context: &EffectContext, _drain: bool) -> Vec<f32> {
        self.ensure_state(context);
        if !self.enabled || self.mix <= 0.0 {
            return samples.to_vec();
        }

        // If an impulse response is configured, skip basic reverb in favor of convolution.
        if context.impulse_response_spec.is_some() {
            return samples.to_vec();
        }

        let Some(state) = self.state.as_mut() else {
            return samples.to_vec();
        };

        let amplitude = if self.mix > 0.0 {
            self.mix
        } else {
            self.settings.amplitude
        };

        if samples.is_empty() {
            if _drain && !state.tail.is_empty() {
                let mut out = Vec::with_capacity(state.tail.len());
                while let Some(sample) = state.tail.pop_front() {
                    out.push(sample * amplitude);
                }
                return out;
            }
            return Vec::new();
        }

        let channels = context.channels.max(1) as u16;
        let sample_rate = context.sample_rate.max(1);
        let source = SamplesBuffer::new(channels, sample_rate, samples.to_vec())
            .buffered()
            .reverb(Duration::from_millis(self.settings.duration_ms), amplitude);

        let mut output: Vec<f32> = source.collect();
        let add_len = state.tail.len().min(output.len());
        if add_len > 0 {
            for i in 0..add_len {
                if let Some(sample) = state.tail.get(i) {
                    output[i] += sample * amplitude;
                }
            }
        }

        state.push_samples(samples);

        output
    }

    /// Reset any internal state (none for basic reverb).
    pub fn reset_state(&mut self) {
        if let Some(state) = self.state.as_mut() {
            state.reset();
        }
        self.state = None;
    }

    /// Mutable access to settings.
    pub fn settings_mut(&mut self) -> &mut BasicReverbSettings {
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
            self.state = Some(BasicReverbState::new(delay_samples));
        }
    }
}

#[derive(Clone)]
struct BasicReverbState {
    delay_samples: usize,
    tail: VecDeque<f32>,
}

impl BasicReverbState {
    fn new(delay_samples: usize) -> Self {
        Self {
            delay_samples,
            tail: VecDeque::with_capacity(delay_samples),
        }
    }

    fn reset(&mut self) {
        self.tail.clear();
    }

    fn push_samples(&mut self, samples: &[f32]) {
        if self.delay_samples == 0 {
            self.tail.clear();
            return;
        }
        for sample in samples {
            if self.tail.len() == self.delay_samples {
                self.tail.pop_front();
            }
            self.tail.push_back(*sample);
        }
    }
}

fn delay_samples(sample_rate: u32, channels: usize, duration_ms: u64) -> usize {
    if duration_ms == 0 {
        return 0;
    }
    let ns = duration_ms.saturating_mul(1_000_000);
    let samples =
        ns.saturating_mul(sample_rate as u64) / 1_000_000_000 * channels as u64;
    samples as usize
}
