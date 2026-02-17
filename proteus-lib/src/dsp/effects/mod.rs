//! Chainable DSP effect modules.

use serde::{Deserialize, Serialize};

use crate::dsp::effects::convolution_reverb::ImpulseResponseSpec;

pub mod basic_reverb;
mod biquad;
pub mod compressor;
pub mod convolution_reverb;
pub mod diffusion_reverb;
pub mod distortion;
pub mod gain;
pub mod high_pass;
mod level;
pub mod limiter;
pub mod low_pass;
pub mod multiband_eq;

#[allow(deprecated)]
#[deprecated(note = "Use DelayReverbEffect instead.")]
pub use basic_reverb::BasicReverbEffect;
#[allow(deprecated)]
#[deprecated(note = "Use DelayReverbSettings instead.")]
pub use basic_reverb::BasicReverbSettings;
pub use basic_reverb::{DelayReverbEffect, DelayReverbSettings};
pub use compressor::{CompressorEffect, CompressorSettings};
pub use convolution_reverb::{ConvolutionReverbEffect, ConvolutionReverbSettings};
pub use diffusion_reverb::{DiffusionReverbEffect, DiffusionReverbSettings};
pub use distortion::{DistortionEffect, DistortionSettings};
pub use gain::{GainEffect, GainSettings};
pub use high_pass::{HighPassFilterEffect, HighPassFilterSettings};
pub use limiter::{LimiterEffect, LimiterSettings};
pub use low_pass::{LowPassFilterEffect, LowPassFilterSettings};
pub use multiband_eq::{
    EqPointSettings, HighEdgeFilterSettings, LowEdgeFilterSettings, MultibandEqEffect,
    MultibandEqSettings,
};

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
    #[serde(rename = "DiffusionReverbSettings")]
    DiffusionReverb(DiffusionReverbEffect),
    #[serde(rename = "ConvolutionReverbSettings")]
    ConvolutionReverb(ConvolutionReverbEffect),
    #[serde(rename = "LowPassFilterSettings")]
    LowPassFilter(LowPassFilterEffect),
    #[serde(rename = "HighPassFilterSettings")]
    HighPassFilter(HighPassFilterEffect),
    #[serde(rename = "DistortionSettings")]
    Distortion(DistortionEffect),
    #[serde(rename = "GainSettings")]
    Gain(GainEffect),
    #[serde(rename = "CompressorSettings")]
    Compressor(CompressorEffect),
    #[serde(rename = "LimiterSettings")]
    Limiter(LimiterEffect),
    #[serde(rename = "MultibandEqSettings")]
    MultibandEq(MultibandEqEffect),
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
    #[allow(deprecated)]
    pub fn process(&mut self, samples: &[f32], context: &EffectContext, drain: bool) -> Vec<f32> {
        match self {
            AudioEffect::BasicReverb(effect) => effect.process(samples, context, drain),
            AudioEffect::DelayReverb(effect) => effect.process(samples, context, drain),
            AudioEffect::DiffusionReverb(effect) => effect.process(samples, context, drain),
            AudioEffect::ConvolutionReverb(effect) => effect.process(samples, context, drain),
            AudioEffect::LowPassFilter(effect) => effect.process(samples, context, drain),
            AudioEffect::HighPassFilter(effect) => effect.process(samples, context, drain),
            AudioEffect::Distortion(effect) => effect.process(samples, context, drain),
            AudioEffect::Gain(effect) => effect.process(samples, context, drain),
            AudioEffect::Compressor(effect) => effect.process(samples, context, drain),
            AudioEffect::Limiter(effect) => effect.process(samples, context, drain),
            AudioEffect::MultibandEq(effect) => effect.process(samples, context, drain),
        }
    }

    /// Reset any internal state maintained by the effect.
    #[allow(deprecated)]
    pub fn reset_state(&mut self) {
        match self {
            AudioEffect::BasicReverb(effect) => effect.reset_state(),
            AudioEffect::DelayReverb(effect) => effect.reset_state(),
            AudioEffect::DiffusionReverb(effect) => effect.reset_state(),
            AudioEffect::ConvolutionReverb(effect) => effect.reset_state(),
            AudioEffect::LowPassFilter(effect) => effect.reset_state(),
            AudioEffect::HighPassFilter(effect) => effect.reset_state(),
            AudioEffect::Distortion(effect) => effect.reset_state(),
            AudioEffect::Gain(effect) => effect.reset_state(),
            AudioEffect::Compressor(effect) => effect.reset_state(),
            AudioEffect::Limiter(effect) => effect.reset_state(),
            AudioEffect::MultibandEq(effect) => effect.reset_state(),
        }
    }

    /// Ensure any internal state (e.g., convolution IR) is initialized.
    #[allow(deprecated)]
    pub fn warm_up(&mut self, context: &EffectContext) {
        match self {
            AudioEffect::BasicReverb(_) => {}
            AudioEffect::DelayReverb(_) => {}
            AudioEffect::DiffusionReverb(_) => {}
            AudioEffect::ConvolutionReverb(effect) => {
                let _ = effect.process(&[], context, false);
            }
            AudioEffect::LowPassFilter(_) => {}
            AudioEffect::HighPassFilter(_) => {}
            AudioEffect::Distortion(_) => {}
            AudioEffect::Gain(_) => {}
            AudioEffect::Compressor(_) => {}
            AudioEffect::Limiter(_) => {}
            AudioEffect::MultibandEq(_) => {}
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

    /// Mutable access to the diffusion reverb effect, if present.
    pub fn as_diffusion_reverb_mut(&mut self) -> Option<&mut DiffusionReverbEffect> {
        match self {
            AudioEffect::DiffusionReverb(effect) => Some(effect),
            _ => None,
        }
    }

    /// Immutable access to the diffusion reverb effect, if present.
    pub fn as_diffusion_reverb(&self) -> Option<&DiffusionReverbEffect> {
        match self {
            AudioEffect::DiffusionReverb(effect) => Some(effect),
            _ => None,
        }
    }

    /// Mutable access to the delay reverb effect, if present.
    #[allow(deprecated)]
    pub fn as_delay_reverb_mut(&mut self) -> Option<&mut DelayReverbEffect> {
        match self {
            AudioEffect::DelayReverb(effect) => Some(effect),
            AudioEffect::BasicReverb(effect) => Some(effect),
            _ => None,
        }
    }

    /// Immutable access to the delay reverb effect, if present.
    #[allow(deprecated)]
    pub fn as_delay_reverb(&self) -> Option<&DelayReverbEffect> {
        match self {
            AudioEffect::DelayReverb(effect) => Some(effect),
            AudioEffect::BasicReverb(effect) => Some(effect),
            _ => None,
        }
    }

    /// Mutable access to the basic reverb effect, if present.
    #[deprecated(note = "Use as_delay_reverb_mut instead.")]
    #[allow(deprecated)]
    pub fn as_basic_reverb_mut(&mut self) -> Option<&mut BasicReverbEffect> {
        self.as_delay_reverb_mut()
    }

    /// Immutable access to the basic reverb effect, if present.
    #[deprecated(note = "Use as_delay_reverb instead.")]
    #[allow(deprecated)]
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
            AudioEffect::DiffusionReverb(DiffusionReverbEffect::default()),
            AudioEffect::ConvolutionReverb(ConvolutionReverbEffect::default()),
            AudioEffect::LowPassFilter(LowPassFilterEffect::default()),
            AudioEffect::HighPassFilter(HighPassFilterEffect::default()),
            AudioEffect::Distortion(DistortionEffect::default()),
            AudioEffect::Gain(GainEffect::default()),
            AudioEffect::Compressor(CompressorEffect::default()),
            AudioEffect::Limiter(LimiterEffect::default()),
            AudioEffect::MultibandEq(MultibandEqEffect::default()),
        ];

        let json = serde_json::to_string(&effects).expect("serialize effects");
        let decoded: Vec<AudioEffect> = serde_json::from_str(&json).expect("deserialize effects");
        assert_eq!(decoded.len(), effects.len());
    }

    #[test]
    fn audio_effect_serde_accepts_aliases() {
        let json = r#"
        [
            {"ConvolutionReverbSettings":{"enabled":true,"wet_dry":0.25}},
            {"DelayReverbSettings":{"enabled":true,"dry_wet":0.5}},
            {"BasicReverbSettings":{"enabled":true,"dry_wet":0.5}},
            {"DiffusionReverbSettings":{"enabled":true,"dry_wet":0.35}},
            {"LowPassFilterSettings":{"enabled":true,"freq":800,"bandwidth":0.7}},
            {"HighPassFilterSettings":{"enabled":true,"frequency_hz":1200,"q":0.9}},
            {"DistortionSettings":{"enabled":true,"gain":2.0,"threshold":0.4}},
            {"GainSettings":{"enabled":true,"gain":1.25}},
            {"CompressorSettings":{"enabled":true,"threshold":-12.0,"ratio":2.0,
                "attack":5.0,"release":50.0,"makeup_db":3.0}},
            {"LimiterSettings":{"enabled":true,"threshold_db":-3.0,"knee_width":2.0,
                "attack_ms":3.0,"release_ms":30.0}},
            {"MultibandEqSettings":{
                "enabled":true,
                "points":[
                    {"freq_hz":120,"q":0.8,"gain_db":3.0},
                    {"freq_hz":1000,"q":1.0,"gain_db":-2.0},
                    {"freq_hz":8000,"q":0.8,"gain_db":2.5}
                ],
                "low_edge":{"type":"high_pass","freq_hz":60,"q":0.7},
                "high_edge":{"type":"high_shelf","freq_hz":10000,"q":0.8,"gain_db":1.5}
            }}
        ]
        "#;

        let decoded: Vec<AudioEffect> = serde_json::from_str(json).expect("deserialize effects");
        assert_eq!(decoded.len(), 11);
    }
}
