//! Chainable DSP effect modules.

use serde::{Deserialize, Serialize};

use crate::container::prot::ImpulseResponseSpec;

pub mod convolution_reverb;

pub use convolution_reverb::{ConvolutionReverbEffect, ConvolutionReverbSettings};

/// Shared context for preparing and running DSP effects.
#[derive(Debug, Clone)]
pub struct EffectContext {
    pub sample_rate: u32,
    pub channels: usize,
    pub container_path: Option<String>,
    pub impulse_response_spec: Option<ImpulseResponseSpec>,
    pub impulse_response_tail_db: f32,
}

/// Configured audio effect that can process interleaved samples.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AudioEffect {
    #[serde(rename = "ConvolutionReverbSettings")]
    ConvolutionReverb(ConvolutionReverbEffect),
}

impl AudioEffect {
    /// Process the provided samples through the effect.
    ///
    /// # Arguments
    /// - `samples`: Interleaved input samples.
    /// - `context`: Environment details (sample rate, channels, etc.).
    /// - `drain`: When true, flush any buffered tail data.
    ///
    /// # Returns
    /// Processed interleaved samples.
    pub fn process(
        &mut self,
        samples: &[f32],
        context: &EffectContext,
        drain: bool,
    ) -> Vec<f32> {
        match self {
            AudioEffect::ConvolutionReverb(effect) => effect.process(samples, context, drain),
        }
    }

    /// Reset any internal state maintained by the effect.
    pub fn reset_state(&mut self) {
        match self {
            AudioEffect::ConvolutionReverb(effect) => effect.reset_state(),
        }
    }

    /// Mutable access to the convolution reverb effect, if present.
    pub fn as_convolution_reverb_mut(&mut self) -> Option<&mut ConvolutionReverbEffect> {
        match self {
            AudioEffect::ConvolutionReverb(effect) => Some(effect),
        }
    }

    /// Immutable access to the convolution reverb effect, if present.
    pub fn as_convolution_reverb(&self) -> Option<&ConvolutionReverbEffect> {
        match self {
            AudioEffect::ConvolutionReverb(effect) => Some(effect),
        }
    }
}
