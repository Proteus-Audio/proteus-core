//! Debug-only helpers for mix-thread logging.

#[cfg(feature = "debug")]
use crate::dsp::effects::AudioEffect;

/// Return a stable effect label used in debug boundary logs.
#[cfg(feature = "debug")]
#[allow(deprecated)]
pub(super) fn effect_label(effect: &AudioEffect) -> &'static str {
    match effect {
        AudioEffect::DelayReverb(_) => "DelayReverb",
        AudioEffect::BasicReverb(_) => "BasicReverb",
        AudioEffect::DiffusionReverb(_) => "DiffusionReverb",
        AudioEffect::ConvolutionReverb(_) => "ConvolutionReverb",
        AudioEffect::LowPassFilter(_) => "LowPassFilter",
        AudioEffect::HighPassFilter(_) => "HighPassFilter",
        AudioEffect::Distortion(_) => "Distortion",
        AudioEffect::Gain(_) => "Gain",
        AudioEffect::Compressor(_) => "Compressor",
        AudioEffect::Limiter(_) => "Limiter",
        AudioEffect::MultibandEq(_) => "MultibandEq",
        AudioEffect::Pan(_) => "Pan",
    }
}

#[cfg(all(test, feature = "debug"))]
mod tests {
    use super::effect_label;
    use crate::dsp::effects::{AudioEffect, GainEffect};

    #[test]
    fn effect_label_returns_stable_name() {
        assert_eq!(
            effect_label(&AudioEffect::Gain(GainEffect::default())),
            "Gain"
        );
    }
}
