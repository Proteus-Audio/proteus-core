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
    /// Maximum queued audio in the sink, in milliseconds (`None` = disabled).
    ///
    /// When set, the playback worker blocks the producer once queued output
    /// exceeds this budget. This control is orthogonal to `max_sink_chunks`:
    /// either, both, or neither may be active. When both are active the
    /// stricter effective cap wins.
    pub max_sink_latency_ms: Option<f32>,
    /// Target output slice duration (ms) for sink appends (`None` = full batch).
    ///
    /// When set, post-DSP output is sliced into chunks of approximately this
    /// duration before being sent to the worker thread. This decouples
    /// internal DSP batch size (which may be large for convolution efficiency)
    /// from the granularity of sink appends, giving the time-based latency
    /// budget finer control. Disabled by default to avoid extra overhead in
    /// stability-first playback modes.
    pub output_slice_ms: Option<f32>,
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
            max_sink_latency_ms: None,
            output_slice_ms: None,
        }
    }

    /// Build an opt-in profile for editor-style live effect authoring.
    ///
    /// This keeps the sink backlog shallow and shortens transition fades so
    /// effect tweaks are heard sooner, while leaving diagnostics disabled by
    /// default. The library does not apply this automatically; player-style
    /// apps should opt in only when lower control latency is worth the reduced
    /// buffering headroom.
    pub fn live_authoring() -> Self {
        Self {
            start_buffer_ms: 20.0,
            track_eos_ms: 1000.0,
            start_sink_chunks: 1,
            max_sink_chunks: 2,
            startup_silence_ms: 0.0,
            startup_fade_ms: 80.0,
            seek_fade_out_ms: 20.0,
            seek_fade_in_ms: 50.0,
            inline_effects_transition_ms: 15.0,
            append_jitter_log_ms: 0.0,
            effect_boundary_log: false,
            parameter_ramp_ms: 5.0,
            max_sink_latency_ms: Some(60.0),
            output_slice_ms: Some(30.0),
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
    /// Estimated queued audio in the output sink, in milliseconds.
    ///
    /// After played chunks are drained, this is the sum of remaining
    /// chunk durations. Slightly overestimates because the currently-playing
    /// chunk is counted at its full duration.
    pub queued_sink_ms: f64,
    /// Duration of the most recently appended output chunk, in milliseconds.
    pub output_chunk_ms: f64,
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
        assert_eq!(settings.start_sink_chunks, 0);
        assert_eq!(settings.max_sink_chunks, 0);
        assert_eq!(settings.startup_fade_ms, 150.0);
        assert_eq!(settings.seek_fade_out_ms, 30.0);
        assert_eq!(settings.seek_fade_in_ms, 80.0);
        assert!(!settings.effect_boundary_log);
        assert!(settings.max_sink_latency_ms.is_none());
        assert!(settings.output_slice_ms.is_none());
    }

    #[test]
    fn playback_buffer_settings_live_authoring_profile_is_opt_in() {
        let settings = PlaybackBufferSettings::live_authoring();
        assert_eq!(settings.start_buffer_ms, 20.0);
        assert_eq!(settings.start_sink_chunks, 1);
        assert_eq!(settings.max_sink_chunks, 2);
        assert_eq!(settings.startup_fade_ms, 80.0);
        assert_eq!(settings.seek_fade_out_ms, 20.0);
        assert_eq!(settings.seek_fade_in_ms, 50.0);
        assert_eq!(settings.inline_effects_transition_ms, 15.0);
        assert_eq!(settings.append_jitter_log_ms, 0.0);
        assert_eq!(settings.parameter_ramp_ms, 5.0);
        assert_eq!(settings.max_sink_latency_ms, Some(60.0));
        assert_eq!(settings.output_slice_ms, Some(30.0));
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
        assert_eq!(metrics.queued_sink_ms, 0.0);
        assert_eq!(metrics.output_chunk_ms, 0.0);
    }
}
