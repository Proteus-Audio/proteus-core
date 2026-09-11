//! Shared policy for estimating and draining audible effect tails.

/// Absolute sample amplitude considered inaudible while draining effect state.
///
/// This is intentionally shared by runtime drain and tail estimates so an
/// advertised audible duration cannot end before the normal drain policy.
pub const DRAIN_SILENCE_EPSILON: f32 = 1.0e-6;

/// Consecutive silent drain passes required before the mix runtime stops.
pub const DRAIN_SILENT_PASSES_TO_STOP: usize = 2;

/// Safety cap for mix-runtime effect-tail drain passes.
pub const MAX_EFFECT_DRAIN_PASSES: usize = 1024;

/// An individual effect's contribution to an audible-duration extension.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectTailEstimate {
    /// Stable display label for the effect contributing the extension.
    pub effect_name: &'static str,
    /// Conservative tail extension in seconds.
    pub seconds: f64,
}

/// A diagnostic breakdown of a serial effect chain's audible extension.
///
/// Tail-producing effects are summed in chain order. This is conservative:
/// a later effect can process the preceding effect's tail, while independent
/// tails may overlap in actual audio. Effects returning an unknown estimate
/// cause the whole chain estimate to be unknown rather than under-reporting.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectChainTailEstimate {
    /// Contributions from effects with a non-zero estimated tail.
    pub per_effect: Vec<EffectTailEstimate>,
    /// Sum of [`Self::per_effect`] in seconds.
    pub total_seconds: f64,
}
