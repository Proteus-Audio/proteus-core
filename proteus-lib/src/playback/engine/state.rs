//! Shared playback state and metrics structures.

/// Buffering configuration for the playback engine.
#[derive(Debug, Clone, Copy)]
pub struct PlaybackBufferSettings {
    pub start_buffer_ms: f32,
    pub track_eos_ms: f32,
    pub start_sink_chunks: usize,
    pub startup_silence_ms: f32,
    pub startup_fade_ms: f32,
}

impl PlaybackBufferSettings {
    /// Create new buffer settings with a given start buffer size (ms).
    pub fn new(start_buffer_ms: f32) -> Self {
        Self {
            start_buffer_ms: start_buffer_ms.max(0.0),
            track_eos_ms: 1000.0,
            start_sink_chunks: 0,
            startup_silence_ms: 0.0,
            startup_fade_ms: 150.0,
        }
    }
}

/// Aggregated DSP chain performance metrics used by debug UI.
#[derive(Debug, Clone, Copy, Default)]
pub struct DspChainMetrics {
    pub dsp_time_ms: f64,
    pub audio_time_ms: f64,
    pub rt_factor: f64,
    pub overrun: bool,
    pub overrun_ms: f64,
    pub avg_overrun_ms: f64,
    pub max_overrun_ms: f64,
    pub avg_dsp_ms: f64,
    pub avg_audio_ms: f64,
    pub avg_rt_factor: f64,
    pub min_rt_factor: f64,
    pub max_rt_factor: f64,
    pub track_key_count: usize,
    pub finished_track_count: usize,
    pub prot_key_count: usize,
    pub chain_ksps: f64,
    pub avg_chain_ksps: f64,
    pub min_chain_ksps: f64,
    pub max_chain_ksps: f64,
    pub underrun_count: u64,
    pub underrun_active: bool,
    pub pop_count: u64,
    pub clip_count: u64,
    pub nan_count: u64,
}
