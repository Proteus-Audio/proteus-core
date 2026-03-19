use super::{DRAIN_SILENCE_EPSILON, DRAIN_SILENT_PASSES_TO_STOP, MAX_EFFECT_DRAIN_PASSES};

#[test]
fn drain_constants_are_positive() {
    assert!(MAX_EFFECT_DRAIN_PASSES > 0);
    assert!(DRAIN_SILENT_PASSES_TO_STOP > 0);
    assert!(DRAIN_SILENCE_EPSILON > 0.0);
}
