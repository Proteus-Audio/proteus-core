//! Shared constants for DSP and playback defaults.

/// Default sample rate used by the library (Hz).
///
/// This is used for synthetic benchmarks and as a fallback in components
/// that do not provide their own sample rate.
pub const SAMPLE_RATE: usize = 44100;
