//! Internal DSP helper primitives shared across effect modules.

pub(crate) mod biquad;
pub(crate) mod level;

use super::EffectContext;

/// Shared behaviour implemented by every DSP effect.
///
/// This trait unifies the processing interface so the `AudioEffect` enum can
/// dispatch generically rather than repeating match arms for every variant.
pub(crate) trait DspEffect {
    /// Process interleaved samples through the effect.
    ///
    /// # Arguments
    /// - `samples`: Interleaved input samples.
    /// - `context`: Environment details (sample rate, channels, etc.).
    /// - `drain`: When true, flush any buffered tail data.
    ///
    /// # Returns
    /// Processed interleaved samples.
    fn process(&mut self, samples: &[f32], context: &EffectContext, drain: bool) -> Vec<f32>;

    /// Reset any internal state maintained by the effect.
    fn reset_state(&mut self);

    /// Ensure any internal state (e.g. convolution IR) is initialized.
    ///
    /// The default implementation is a no-op. Override for effects that
    /// require eager initialization before the first `process` call.
    fn warm_up(&mut self, _context: &EffectContext) {}
}

#[cfg(test)]
mod tests {
    use crate::dsp::effects::core::DspEffect;

    #[derive(Default)]
    struct DummyEffect {
        processed: usize,
        reset_called: bool,
    }

    impl super::DspEffect for DummyEffect {
        fn process(
            &mut self,
            samples: &[f32],
            _context: &super::EffectContext,
            _drain: bool,
        ) -> Vec<f32> {
            self.processed += samples.len();
            samples.to_vec()
        }

        fn reset_state(&mut self) {
            self.reset_called = true;
            self.processed = 0;
        }
    }

    #[test]
    fn default_warm_up_is_noop_and_trait_methods_work() {
        let mut effect = DummyEffect::default();

        let context = super::EffectContext::new(48_000, 2, None, None, -60.0).unwrap();

        let out = effect.process(&[0.1, 0.2, 0.3, 0.4], &context, false);
        assert_eq!(out, vec![0.1, 0.2, 0.3, 0.4]);
        assert_eq!(effect.processed, 4);

        effect.warm_up(&context);
        assert_eq!(effect.processed, 4);

        effect.reset_state();
        assert!(effect.reset_called);
        assert_eq!(effect.processed, 0);
    }
}
