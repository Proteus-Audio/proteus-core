#[derive(Debug, Clone, Copy)]
pub struct ReverbSettings {
    pub enabled: bool,
    pub dry_wet: f32,
    pub reset_pending: bool,
}

impl ReverbSettings {
    pub fn new(dry_wet: f32) -> Self {
        Self {
            enabled: true,
            dry_wet: dry_wet.clamp(0.0, 1.0),
            reset_pending: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PlaybackBufferSettings {
    pub start_buffer_ms: f32,
}

impl PlaybackBufferSettings {
    pub fn new(start_buffer_ms: f32) -> Self {
        Self {
            start_buffer_ms: start_buffer_ms.max(0.0),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ReverbMetrics {
    pub dsp_time_ms: f64,
    pub audio_time_ms: f64,
    pub rt_factor: f64,
    pub avg_dsp_ms: f64,
    pub avg_audio_ms: f64,
    pub avg_rt_factor: f64,
    pub min_rt_factor: f64,
    pub max_rt_factor: f64,
    pub buffer_fill: f64,
    pub avg_buffer_fill: f64,
    pub min_buffer_fill: f64,
    pub max_buffer_fill: f64,
    pub chain_time_ms: f64,
    pub avg_chain_time_ms: f64,
    pub min_chain_time_ms: f64,
    pub max_chain_time_ms: f64,
    pub out_interval_ms: f64,
    pub avg_out_interval_ms: f64,
    pub min_out_interval_ms: f64,
    pub max_out_interval_ms: f64,
    pub wake_total: u64,
    pub wake_idle: u64,
}
