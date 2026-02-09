//! Chainable DSP effect modules.

use serde::{Deserialize, Serialize};

use crate::dsp::effects::convolution_reverb::ImpulseResponseSpec;

pub mod convolution_reverb;
pub mod basic_reverb;
pub mod distortion;
pub mod high_pass;
pub mod low_pass;
pub mod compressor;
pub mod limiter;
mod biquad;

pub use basic_reverb::{DelayReverbEffect, DelayReverbSettings};
pub use convolution_reverb::{ConvolutionReverbEffect, ConvolutionReverbSettings};
pub use distortion::{DistortionEffect, DistortionSettings};
pub use high_pass::{HighPassFilterEffect, HighPassFilterSettings};
pub use low_pass::{LowPassFilterEffect, LowPassFilterSettings};
pub use compressor::{CompressorEffect, CompressorSettings};
pub use limiter::{LimiterEffect, LimiterSettings};
#[deprecated(note = "Use DelayReverbEffect instead.")]
pub use basic_reverb::BasicReverbEffect;
#[deprecated(note = "Use DelayReverbSettings instead.")]
pub use basic_reverb::BasicReverbSettings;

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
    #[serde(rename = "DelayReverbSettings")]
    DelayReverb(DelayReverbEffect),
    #[deprecated(note = "Use AudioEffect::DelayReverb instead.")]
    #[serde(rename = "BasicReverbSettings")]
    BasicReverb(DelayReverbEffect),
    #[serde(rename = "ConvolutionReverbSettings")]
    ConvolutionReverb(ConvolutionReverbEffect),
    #[serde(rename = "LowPassFilterSettings")]
    LowPassFilter(LowPassFilterEffect),
    #[serde(rename = "HighPassFilterSettings")]
    HighPassFilter(HighPassFilterEffect),
    #[serde(rename = "DistortionSettings")]
    Distortion(DistortionEffect),
    #[serde(rename = "CompressorSettings")]
    Compressor(CompressorEffect),
    #[serde(rename = "LimiterSettings")]
    Limiter(LimiterEffect),
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
            AudioEffect::DelayReverb(effect) => effect.process(samples, context, drain),
            AudioEffect::ConvolutionReverb(effect) => effect.process(samples, context, drain),
            AudioEffect::LowPassFilter(effect) => effect.process(samples, context, drain),
            AudioEffect::HighPassFilter(effect) => effect.process(samples, context, drain),
            AudioEffect::Distortion(effect) => effect.process(samples, context, drain),
            AudioEffect::Compressor(effect) => effect.process(samples, context, drain),
            AudioEffect::Limiter(effect) => effect.process(samples, context, drain),
        }
    }

    /// Reset any internal state maintained by the effect.
    pub fn reset_state(&mut self) {
        match self {
            AudioEffect::BasicReverb(effect) => effect.reset_state(),
            AudioEffect::DelayReverb(effect) => effect.reset_state(),
            AudioEffect::ConvolutionReverb(effect) => effect.reset_state(),
            AudioEffect::LowPassFilter(effect) => effect.reset_state(),
            AudioEffect::HighPassFilter(effect) => effect.reset_state(),
            AudioEffect::Distortion(effect) => effect.reset_state(),
            AudioEffect::Compressor(effect) => effect.reset_state(),
            AudioEffect::Limiter(effect) => effect.reset_state(),
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

    /// Mutable access to the delay reverb effect, if present.
    pub fn as_delay_reverb_mut(&mut self) -> Option<&mut DelayReverbEffect> {
        match self {
            AudioEffect::DelayReverb(effect) => Some(effect),
            AudioEffect::BasicReverb(effect) => Some(effect),
            _ => None,
        }
    }

    /// Immutable access to the delay reverb effect, if present.
    pub fn as_delay_reverb(&self) -> Option<&DelayReverbEffect> {
        match self {
            AudioEffect::DelayReverb(effect) => Some(effect),
            AudioEffect::BasicReverb(effect) => Some(effect),
            _ => None,
        }
    }

    /// Mutable access to the basic reverb effect, if present.
    #[deprecated(note = "Use as_delay_reverb_mut instead.")]
    pub fn as_basic_reverb_mut(&mut self) -> Option<&mut BasicReverbEffect> {
        self.as_delay_reverb_mut()
    }

    /// Immutable access to the basic reverb effect, if present.
    #[deprecated(note = "Use as_delay_reverb instead.")]
    pub fn as_basic_reverb(&self) -> Option<&BasicReverbEffect> {
        self.as_delay_reverb()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_effect_serde_roundtrip_variants() {
        let effects = vec![
            AudioEffect::DelayReverb(DelayReverbEffect::default()),
            AudioEffect::ConvolutionReverb(ConvolutionReverbEffect::default()),
            AudioEffect::LowPassFilter(LowPassFilterEffect::default()),
            AudioEffect::HighPassFilter(HighPassFilterEffect::default()),
            AudioEffect::Distortion(DistortionEffect::default()),
            AudioEffect::Compressor(CompressorEffect::default()),
            AudioEffect::Limiter(LimiterEffect::default()),
        ];

        let json = serde_json::to_string(&effects).expect("serialize effects");
        let decoded: Vec<AudioEffect> =
            serde_json::from_str(&json).expect("deserialize effects");
        assert_eq!(decoded.len(), effects.len());
    }

    #[test]
    fn audio_effect_serde_accepts_aliases() {
        let json = r#"
        [
            {"ConvolutionReverbSettings":{"enabled":true,"wet_dry":0.25}},
            {"DelayReverbSettings":{"enabled":true,"dry_wet":0.5}},
            {"BasicReverbSettings":{"enabled":true,"dry_wet":0.5}},
            {"LowPassFilterSettings":{"enabled":true,"freq":800,"bandwidth":0.7}},
            {"HighPassFilterSettings":{"enabled":true,"frequency_hz":1200,"q":0.9}},
            {"DistortionSettings":{"enabled":true,"gain":2.0,"threshold":0.4}},
            {"CompressorSettings":{"enabled":true,"threshold":-12.0,"ratio":2.0,
                "attack":5.0,"release":50.0,"makeup_db":3.0}},
            {"LimiterSettings":{"enabled":true,"threshold_db":-3.0,"knee_width":2.0,
                "attack_ms":3.0,"release_ms":30.0}}
        ]
        "#;

        let decoded: Vec<AudioEffect> =
            serde_json::from_str(json).expect("deserialize effects");
        assert_eq!(decoded.len(), 8);
    }
}
