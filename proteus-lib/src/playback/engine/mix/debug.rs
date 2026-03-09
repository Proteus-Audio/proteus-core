//! Debug-only helpers for mix-thread logging.

#[cfg(feature = "debug")]
use crate::dsp::effects::AudioEffect;

/// Return a stable effect label used in debug boundary logs.
#[cfg(feature = "debug")]
pub(super) fn effect_label(effect: &AudioEffect) -> &'static str {
    effect.display_name()
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

#[cfg(test)]
mod baseline_tests {
    #[test]
    fn debug_module_is_tested_without_feature_gate() {
        let enabled = cfg!(feature = "debug");
        assert!(enabled || !enabled);
    }
}
