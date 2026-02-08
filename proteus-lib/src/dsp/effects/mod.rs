//! Chainable DSP effect modules.

use serde::{Deserialize, Serialize};

use crate::dsp::effects::convolution_reverb::ImpulseResponseSpec;

pub mod convolution_reverb;
pub mod basic_reverb;
pub mod distortion;
pub mod high_pass;
pub mod low_pass;
mod biquad;

pub use basic_reverb::{BasicReverbEffect, BasicReverbSettings};
pub use convolution_reverb::{ConvolutionReverbEffect, ConvolutionReverbSettings};
pub use distortion::{DistortionEffect, DistortionSettings};
pub use high_pass::{HighPassFilterEffect, HighPassFilterSettings};
pub use low_pass::{LowPassFilterEffect, LowPassFilterSettings};

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
    #[serde(rename = "BasicReverbSettings")]
    BasicReverb(BasicReverbEffect),
    #[serde(rename = "ConvolutionReverbSettings")]
    ConvolutionReverb(ConvolutionReverbEffect),
    #[serde(rename = "LowPassFilterSettings")]
    LowPassFilter(LowPassFilterEffect),
    #[serde(rename = "HighPassFilterSettings")]
    HighPassFilter(HighPassFilterEffect),
    #[serde(rename = "DistortionSettings")]
    Distortion(DistortionEffect),
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
            AudioEffect::BasicReverb(effect) => effect.process(samples, context, drain),
            AudioEffect::ConvolutionReverb(effect) => effect.process(samples, context, drain),
            AudioEffect::LowPassFilter(effect) => effect.process(samples, context, drain),
            AudioEffect::HighPassFilter(effect) => effect.process(samples, context, drain),
            AudioEffect::Distortion(effect) => effect.process(samples, context, drain),
        }
    }

    /// Reset any internal state maintained by the effect.
    pub fn reset_state(&mut self) {
        match self {
            AudioEffect::BasicReverb(effect) => effect.reset_state(),
            AudioEffect::ConvolutionReverb(effect) => effect.reset_state(),
            AudioEffect::LowPassFilter(effect) => effect.reset_state(),
            AudioEffect::HighPassFilter(effect) => effect.reset_state(),
            AudioEffect::Distortion(effect) => effect.reset_state(),
        }
    }

    /// Mutable access to the convolution reverb effect, if present.
    pub fn as_convolution_reverb_mut(&mut self) -> Option<&mut ConvolutionReverbEffect> {
        match self {
            AudioEffect::ConvolutionReverb(effect) => Some(effect),
            _ => None,
        }
    }

    /// Immutable access to the convolution reverb effect, if present.
    pub fn as_convolution_reverb(&self) -> Option<&ConvolutionReverbEffect> {
        match self {
            AudioEffect::ConvolutionReverb(effect) => Some(effect),
            _ => None,
        }
    }

    /// Mutable access to the basic reverb effect, if present.
    pub fn as_basic_reverb_mut(&mut self) -> Option<&mut BasicReverbEffect> {
        match self {
            AudioEffect::BasicReverb(effect) => Some(effect),
            _ => None,
        }
    }

    /// Immutable access to the basic reverb effect, if present.
    pub fn as_basic_reverb(&self) -> Option<&BasicReverbEffect> {
        match self {
            AudioEffect::BasicReverb(effect) => Some(effect),
            _ => None,
        }
    }
}
