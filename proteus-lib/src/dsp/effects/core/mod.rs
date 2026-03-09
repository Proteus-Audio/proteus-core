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
