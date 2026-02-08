//! High-pass filter effect based on rodio's BLT filter.

use serde::{Deserialize, Serialize};

use super::biquad::{BiquadKind, BiquadState};
use super::EffectContext;

const DEFAULT_FREQ_HZ: u32 = 1000;
const DEFAULT_Q: f32 = 0.5;

/// Serialized configuration for high-pass filter parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HighPassFilterSettings {
    #[serde(alias = "freq", alias = "frequency_hz")]
    pub freq_hz: u32,
    #[serde(alias = "bandwidth")]
    pub q: f32,
}

impl HighPassFilterSettings {
    /// Create a high-pass settings payload.
    pub fn new(freq_hz: u32, q: f32) -> Self {
        Self { freq_hz, q }
    }
}

impl Default for HighPassFilterSettings {
    fn default() -> Self {
        Self {
            freq_hz: DEFAULT_FREQ_HZ,
            q: DEFAULT_Q,
        }
    }
}

/// Configured high-pass filter effect with runtime state.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HighPassFilterEffect {
    pub enabled: bool,
    #[serde(flatten)]
    pub settings: HighPassFilterSettings,
    #[serde(skip)]
    state: Option<BiquadState>,
}

impl std::fmt::Debug for HighPassFilterEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HighPassFilterEffect")
            .field("enabled", &self.enabled)
            .field("settings", &self.settings)
            .finish()
    }
}

impl Default for HighPassFilterEffect {
    fn default() -> Self {
        Self {
            enabled: false,
            settings: HighPassFilterSettings::default(),
            state: None,
        }
    }
}

impl HighPassFilterEffect {
    /// Process interleaved samples through the high-pass filter.
    ///
    /// # Arguments
    /// - `samples`: Interleaved input samples.
    /// - `context`: Environment details (sample rate, channels, etc.).
    /// - `drain`: Unused for this effect.
    ///
    /// # Returns
    /// Processed interleaved samples.
    pub fn process(&mut self, samples: &[f32], context: &EffectContext, _drain: bool) -> Vec<f32> {
        if !self.enabled {
            return samples.to_vec();
        }

        self.ensure_state(context);
        let Some(state) = self.state.as_mut() else {
            return samples.to_vec();
        };

        state.process(samples)
    }

    /// Reset any internal state held by the filter.
    pub fn reset_state(&mut self) {
        if let Some(state) = self.state.as_mut() {
            state.reset();
        }
        self.state = None;
    }

    fn ensure_state(&mut self, context: &EffectContext) {
        let needs_reset = self
            .state
            .as_ref()
            .map(|state| {
                !state.matches(
                    BiquadKind::HighPass,
                    context.sample_rate,
                    context.channels,
                    self.settings.freq_hz,
                    self.settings.q,
                )
            })
            .unwrap_or(true);

        if needs_reset {
            self.state = Some(BiquadState::new(
                BiquadKind::HighPass,
                context.sample_rate,
                context.channels,
                self.settings.freq_hz,
                self.settings.q,
            ));
        }
    }
}
