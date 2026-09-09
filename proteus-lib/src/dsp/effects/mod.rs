//! Chainable DSP effect modules.
//!
//! ## Internal Module Layout Convention
//! - Each effect implements the `DspEffect` trait (in the private `core` module), which
//!   provides the canonical `process`, `reset_state`, and `warm_up` methods.
//! - Split reusable algorithmic components into sibling submodules (for
//!   example `convolution`, `impulse_response`, `reverb`) when complexity
//!   grows.
//! - Prefer one effect per directory when internals exceed a single-file scope.

use serde::{Deserialize, Serialize};

use crate::dsp::effects::convolution_reverb::ImpulseResponseSpec;
use crate::dsp::effects::core::smoother;
use crate::dsp::meter::FilterResponseCurve;

pub mod basic_reverb;
pub mod compressor;
pub mod convolution_reverb;
mod core;
pub mod diffusion_reverb;
pub mod distortion;
pub mod gain;
pub mod high_pass;
pub mod limiter;
pub mod low_pass;
pub mod multiband_eq;
pub mod pan;

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
pub use pan::{PanEffect, PanSettings};

/// Error returned when constructing an [`EffectContext`] with invalid parameters.
#[derive(Debug, Clone)]
pub enum EffectContextError {
    /// Sample rate must be greater than zero.
    ZeroSampleRate,
    /// Channel count must be at least one.
    ZeroChannels,
}

impl std::fmt::Display for EffectContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroSampleRate => write!(f, "sample rate must be greater than zero"),
            Self::ZeroChannels => write!(f, "channel count must be at least one"),
        }
    }
}

impl std::error::Error for EffectContextError {}

/// Shared context for preparing and running DSP effects.
#[derive(Debug, Clone)]
pub struct EffectContext {
    sample_rate: u32,
    channels: usize,
    container_path: Option<String>,
    impulse_response_spec: Option<ImpulseResponseSpec>,
    impulse_response_tail_db: f32,
    parameter_ramp_samples: usize,
}

impl EffectContext {
    /// Create a new effect context after validating invariants.
    ///
    /// # Arguments
    ///
    /// * `sample_rate` - Sample rate of the audio stream in Hz; must be > 0.
    /// * `channels` - Number of interleaved audio channels; must be >= 1.
    /// * `container_path` - Optional path to the loaded container file.
    /// * `impulse_response_spec` - Optional IR spec for convolution reverb.
    /// * `impulse_response_tail_db` - dB level below peak for tail silence detection.
    ///
    /// # Errors
    ///
    /// Returns [`EffectContextError`] if `sample_rate` is zero or `channels` is zero.
    pub fn new(
        sample_rate: u32,
        channels: usize,
        container_path: Option<String>,
        impulse_response_spec: Option<ImpulseResponseSpec>,
        impulse_response_tail_db: f32,
    ) -> Result<Self, EffectContextError> {
        if sample_rate == 0 {
            return Err(EffectContextError::ZeroSampleRate);
        }
        if channels == 0 {
            return Err(EffectContextError::ZeroChannels);
        }
        Ok(Self {
            sample_rate,
            channels,
            container_path,
            impulse_response_spec,
            impulse_response_tail_db,
            parameter_ramp_samples: smoother::ramp_samples(
                smoother::DEFAULT_PARAMETER_RAMP_MS,
                sample_rate,
            ),
        })
    }

    /// Sample rate of the audio stream, in Hz.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Number of interleaved audio channels in each sample buffer.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Filesystem path to the loaded container, if any.
    pub fn container_path(&self) -> Option<&str> {
        self.container_path.as_deref()
    }

    /// Specification for the impulse response used by convolution-reverb effects.
    pub fn impulse_response_spec(&self) -> Option<&ImpulseResponseSpec> {
        self.impulse_response_spec.as_ref()
    }

    /// dB level below peak at which the impulse response tail is considered silent.
    pub fn impulse_response_tail_db(&self) -> f32 {
        self.impulse_response_tail_db
    }

    /// Number of samples over which parameter changes should be linearly ramped.
    pub fn parameter_ramp_samples(&self) -> usize {
        self.parameter_ramp_samples
    }

    /// Override the parameter ramp duration.
    pub fn set_parameter_ramp_ms(&mut self, ms: f32) {
        self.parameter_ramp_samples = smoother::ramp_samples(ms.max(0.0), self.sample_rate);
    }
}

// ---------------------------------------------------------------------------
// Macro: generates the `AudioEffect` enum and its core dispatch methods from
// a single declaration.  Adding a new effect only requires one new entry here
// (plus the module, re-export, and trait impl in the effect file).
// ---------------------------------------------------------------------------

macro_rules! define_audio_effects {
    (
        effects {
            $( $variant:ident($effect_ty:ident, $serde_name:literal $(, aliases = [$($serde_alias:literal),* $(,)?])? ) ),* $(,)?
        }
    ) => {
        /// Configured audio effect that can process interleaved samples.
        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub enum AudioEffect {
            $(
                /// Effect variant wrapping a [`
                #[doc = stringify!($effect_ty)]
                /// `] configuration and runtime state.
                #[serde(rename = $serde_name)]
                $( $( #[serde(alias = $serde_alias)] )* )?
                $variant($effect_ty),
            )*
        }

        impl AudioEffect {
            /// Preserve the historical alias-normalization hook for runtime callers.
            pub fn normalize_legacy_alias(self) -> Self {
                self
            }

            /// Canonical display label shared across CLI and runtime debug surfaces.
            pub fn display_name(&self) -> &'static str {
                match self {
                    $( AudioEffect::$variant(_) => stringify!($variant), )*
                }
            }

            /// Return a mutable reference to the inner effect as a trait object.
            fn as_dsp_effect(&mut self) -> &mut dyn core::DspEffect {
                match self {
                    $( AudioEffect::$variant(effect) => effect, )*
                }
            }

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
                self.as_dsp_effect().process(samples, context, drain)
            }

            /// Process the provided samples through the effect, appending output to `output`.
            ///
            /// # Arguments
            /// - `input`: Interleaved input samples.
            /// - `output`: Caller-owned buffer to append processed samples into; clear before
            ///   calling if a fresh result is needed.
            /// - `context`: Environment details (sample rate, channels, etc.).
            /// - `drain`: When true, flush any buffered tail data.
            pub fn process_into(
                &mut self,
                input: &[f32],
                output: &mut Vec<f32>,
                context: &EffectContext,
                drain: bool,
            ) {
                self.as_dsp_effect().process_into(input, output, context, drain)
            }

            /// Reset any internal state maintained by the effect.
            pub fn reset_state(&mut self) {
                self.as_dsp_effect().reset_state();
            }

            /// Ensure any internal state (e.g., convolution IR) is initialized.
            pub fn warm_up(&mut self, context: &EffectContext) {
                self.as_dsp_effect().warm_up(context);
            }
        }
    };
}

define_audio_effects! {
    effects {
        DelayReverb(DelayReverbEffect, "DelayReverbSettings", aliases = ["BasicReverbSettings"]),
        DiffusionReverb(DiffusionReverbEffect, "DiffusionReverbSettings"),
        ConvolutionReverb(ConvolutionReverbEffect, "ConvolutionReverbSettings"),
        LowPassFilter(LowPassFilterEffect, "LowPassFilterSettings"),
        HighPassFilter(HighPassFilterEffect, "HighPassFilterSettings"),
        Distortion(DistortionEffect, "DistortionSettings"),
        Gain(GainEffect, "GainSettings"),
        Compressor(CompressorEffect, "CompressorSettings"),
        Limiter(LimiterEffect, "LimiterSettings"),
        MultibandEq(MultibandEqEffect, "MultibandEqSettings"),
        Pan(PanEffect, "PanSettings"),
    }
}

// --- Variant-specific accessors (not generated by the macro) ---------------

impl AudioEffect {
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
    pub fn as_delay_reverb_mut(&mut self) -> Option<&mut DelayReverbEffect> {
        match self {
            AudioEffect::DelayReverb(effect) => Some(effect),
            _ => None,
        }
    }

    /// Immutable access to the delay reverb effect, if present.
    pub fn as_delay_reverb(&self) -> Option<&DelayReverbEffect> {
        match self {
            AudioEffect::DelayReverb(effect) => Some(effect),
            _ => None,
        }
    }

    /// Build an analytical frequency-response curve from the effect settings.
    ///
    /// Builds without the `effect-meter` feature return `None` for every slot.
    pub fn frequency_response_curve(
        &self,
        sample_rate: u32,
        num_points: usize,
    ) -> Option<FilterResponseCurve> {
        #[cfg(feature = "effect-meter")]
        {
            match self {
                AudioEffect::LowPassFilter(effect) => {
                    Some(effect.frequency_response_curve(sample_rate, num_points))
                }
                AudioEffect::HighPassFilter(effect) => {
                    Some(effect.frequency_response_curve(sample_rate, num_points))
                }
                AudioEffect::MultibandEq(effect) => {
                    Some(effect.frequency_response_curve(sample_rate, num_points))
                }
                _ => None,
            }
        }

        #[cfg(not(feature = "effect-meter"))]
        {
            let _ = sample_rate;
            let _ = num_points;
            None
        }
    }
}

/// Normalize deprecated effect aliases for runtime processing.
pub fn normalize_legacy_effect_aliases(effects: Vec<AudioEffect>) -> Vec<AudioEffect> {
    effects
        .into_iter()
        .map(AudioEffect::normalize_legacy_alias)
        .collect()
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
            AudioEffect::Pan(PanEffect::default()),
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
            }},
            {"PanSettings":{"enabled":true,"pan":-0.3}}
        ]
        "#;

        let decoded: Vec<AudioEffect> = serde_json::from_str(json).expect("deserialize effects");
        assert_eq!(decoded.len(), 12);
    }

    #[test]
    fn effect_context_new_valid() {
        let ctx = EffectContext::new(48_000, 2, None, None, -60.0);
        assert!(ctx.is_ok());
        let ctx = ctx.unwrap();
        assert_eq!(ctx.sample_rate(), 48_000);
        assert_eq!(ctx.channels(), 2);
        assert!(ctx.container_path().is_none());
        assert!(ctx.impulse_response_spec().is_none());
        assert!((ctx.impulse_response_tail_db() - (-60.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn effect_context_new_with_optional_fields() {
        let ctx = EffectContext::new(
            44_100,
            1,
            Some("/path/to/file.prot".to_string()),
            None,
            -40.0,
        )
        .unwrap();
        assert_eq!(ctx.sample_rate(), 44_100);
        assert_eq!(ctx.channels(), 1);
        assert_eq!(ctx.container_path(), Some("/path/to/file.prot"));
    }

    #[test]
    fn effect_context_rejects_zero_sample_rate() {
        let result = EffectContext::new(0, 2, None, None, -60.0);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            EffectContextError::ZeroSampleRate
        ));
    }

    #[test]
    fn effect_context_rejects_zero_channels() {
        let result = EffectContext::new(48_000, 0, None, None, -60.0);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            EffectContextError::ZeroChannels
        ));
    }

    #[test]
    fn effect_context_clone_preserves_validity() {
        let ctx = EffectContext::new(48_000, 2, None, None, -60.0).unwrap();
        let cloned = ctx.clone();
        assert_eq!(cloned.sample_rate(), 48_000);
        assert_eq!(cloned.channels(), 2);
    }
}
