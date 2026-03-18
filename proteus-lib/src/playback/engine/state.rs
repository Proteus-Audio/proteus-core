//! Shared playback state and metrics structures.

/// Buffering configuration for the playback engine.
#[derive(Debug, Clone, Copy)]
pub struct PlaybackBufferSettings {
    /// Minimum milliseconds each track buffer must hold before playback starts.
    pub start_buffer_ms: f32,
    /// Milliseconds before end-of-stream at which a track is considered finished.
    pub track_eos_ms: f32,
    /// Number of pre-mixed chunks to append to the sink before audio begins.
    pub start_sink_chunks: usize,
    /// Maximum number of mixed chunks allowed in the sink queue at any time.
    pub max_sink_chunks: usize,
    /// Duration of leading silence (in ms) injected before the first audio frame.
    pub startup_silence_ms: f32,
    /// Duration of the fade-in applied at engine startup, in milliseconds.
    pub startup_fade_ms: f32,
    /// Duration of the fade-out applied before a seek operation, in milliseconds.
    pub seek_fade_out_ms: f32,
    /// Duration of the fade-in applied after a seek operation, in milliseconds.
    pub seek_fade_in_ms: f32,
    /// Crossfade duration (ms) used when switching inline effects mid-playback.
    pub inline_effects_transition_ms: f32,
    /// Threshold in milliseconds above which a late-append event is logged.
    pub append_jitter_log_ms: f32,
    /// When `true`, logs a message each time an effect boundary is crossed.
    pub effect_boundary_log: bool,
    /// Duration in milliseconds for per-parameter smoothing ramps (default: 5.0).
    pub parameter_ramp_ms: f32,
}

impl PlaybackBufferSettings {
    /// Create new buffer settings with a given start buffer size (ms).
    pub fn new(start_buffer_ms: f32) -> Self {
        Self {
            start_buffer_ms: start_buffer_ms.max(0.0),
            track_eos_ms: 1000.0,
            start_sink_chunks: 0,
            max_sink_chunks: 0,
            startup_silence_ms: 0.0,
            startup_fade_ms: 150.0,
            seek_fade_out_ms: 30.0,
            seek_fade_in_ms: 80.0,
            inline_effects_transition_ms: 25.0,
            append_jitter_log_ms: 0.0,
            effect_boundary_log: false,
            parameter_ramp_ms: 5.0,
        }
    }
}

/// Aggregated DSP chain performance metrics used by debug UI.
#[derive(Debug, Clone, Copy, Default)]
pub struct DspChainMetrics {
    /// Whether the last mix cycle exceeded its deadline.
    pub overrun: bool,
    /// Duration of the most recent overrun, in milliseconds.
    pub overrun_ms: f64,
    /// Running average overrun duration across all cycles, in milliseconds.
    pub avg_overrun_ms: f64,
    /// Largest overrun duration observed since engine start, in milliseconds.
    pub max_overrun_ms: f64,
    /// Number of active track keys currently managed by the engine.
    pub track_key_count: usize,
    /// Number of track keys that have reached end-of-stream.
    pub finished_track_count: usize,
    /// Number of prot (container) source keys in the current playback plan.
    pub prot_key_count: usize,
    /// DSP throughput for the most recent cycle, in kilo-samples per second.
    pub chain_ksps: f64,
    /// Rolling average DSP throughput, in kilo-samples per second.
    pub avg_chain_ksps: f64,
    /// Minimum DSP throughput observed since engine start, in kilo-samples per second.
    pub min_chain_ksps: f64,
    /// Maximum DSP throughput observed since engine start, in kilo-samples per second.
    pub max_chain_ksps: f64,
    /// Total number of buffer underrun events since engine start.
    pub underrun_count: u64,
    /// Whether a buffer underrun is currently active.
    pub underrun_active: bool,
    /// Total number of inter-chunk discontinuities (pops) detected since engine start.
    pub pop_count: u64,
    /// Total number of clipped samples detected in the output stream since engine start.
    pub clip_count: u64,
    /// Total number of NaN samples detected in the output stream since engine start.
    pub nan_count: u64,
    /// Total number of late-append events recorded since engine start.
    pub late_append_count: u64,
    /// Whether a late-append condition is currently active.
    pub late_append_active: bool,
}

#[cfg(test)]
mod tests {
    use super::{DspChainMetrics, PlaybackBufferSettings};

    #[test]
    fn playback_buffer_settings_clamps_negative_start_buffer() {
        let settings = PlaybackBufferSettings::new(-42.0);
        assert_eq!(settings.start_buffer_ms, 0.0);
    }

    #[test]
    fn playback_buffer_settings_uses_expected_defaults() {
        let settings = PlaybackBufferSettings::new(25.0);
        assert_eq!(settings.start_buffer_ms, 25.0);
        assert_eq!(settings.track_eos_ms, 1000.0);
        assert_eq!(settings.startup_fade_ms, 150.0);
        assert_eq!(settings.seek_fade_out_ms, 30.0);
        assert_eq!(settings.seek_fade_in_ms, 80.0);
        assert!(!settings.effect_boundary_log);
    }

    #[test]
    fn dsp_chain_metrics_default_is_zeroed_and_flags_false() {
        let metrics = DspChainMetrics::default();
        assert!(!metrics.overrun);
        assert_eq!(metrics.overrun_ms, 0.0);
        assert_eq!(metrics.track_key_count, 0);
        assert_eq!(metrics.underrun_count, 0);
        assert!(!metrics.underrun_active);
        assert_eq!(metrics.pop_count, 0);
        assert_eq!(metrics.clip_count, 0);
        assert_eq!(metrics.nan_count, 0);
        assert_eq!(metrics.late_append_count, 0);
        assert!(!metrics.late_append_active);
    }
}
