use std::sync::atomic::Ordering;

use super::{Player, PlayerState};

impl Player {
    /// Configure the minimum buffered audio (ms) before playback starts.
    pub fn set_start_buffer_ms(&self, start_buffer_ms: f32) {
        let mut settings = self.buffer_settings.lock().unwrap();
        settings.start_buffer_ms = start_buffer_ms.max(0.0);
    }

    /// Configure heuristic end-of-track threshold for containers (ms).
    pub fn set_track_eos_ms(&self, track_eos_ms: f32) {
        let mut settings = self.buffer_settings.lock().unwrap();
        settings.track_eos_ms = track_eos_ms.max(0.0);
    }

    /// Configure minimum sink chunks queued before playback starts/resumes.
    pub fn set_start_sink_chunks(&self, chunks: usize) {
        let mut settings = self.buffer_settings.lock().unwrap();
        settings.start_sink_chunks = chunks;
    }

    /// Configure the maximum sink chunks queued before producer backpressure.
    ///
    /// Set to `0` to disable this guard.
    pub fn set_max_sink_chunks(&self, chunks: usize) {
        let mut settings = self.buffer_settings.lock().unwrap();
        settings.max_sink_chunks = chunks;
    }

    /// Configure the startup silence pre-roll (ms).
    pub fn set_startup_silence_ms(&self, ms: f32) {
        let mut settings = self.buffer_settings.lock().unwrap();
        settings.startup_silence_ms = ms.max(0.0);
    }

    /// Configure the startup fade-in length (ms).
    pub fn set_startup_fade_ms(&self, ms: f32) {
        let mut settings = self.buffer_settings.lock().unwrap();
        settings.startup_fade_ms = ms.max(0.0);
    }

    /// Configure seek fade-out length (ms) before restarting playback.
    pub fn set_seek_fade_out_ms(&self, ms: f32) {
        let mut settings = self.buffer_settings.lock().unwrap();
        settings.seek_fade_out_ms = ms.max(0.0);
    }

    /// Configure seek fade-in length (ms) after restarting playback.
    pub fn set_seek_fade_in_ms(&self, ms: f32) {
        let mut settings = self.buffer_settings.lock().unwrap();
        settings.seek_fade_in_ms = ms.max(0.0);
    }

    /// Configure the append jitter logging threshold (ms). 0 disables logging.
    pub fn set_append_jitter_log_ms(&self, ms: f32) {
        let mut settings = self.buffer_settings.lock().unwrap();
        settings.append_jitter_log_ms = ms.max(0.0);
    }

    /// Configure inline effects transition duration (ms) for `set_effects_inline`.
    pub fn set_inline_effects_transition_ms(&self, ms: f32) {
        let mut settings = self.buffer_settings.lock().unwrap();
        settings.inline_effects_transition_ms = ms.max(0.0);
    }

    /// Enable or disable per-effect boundary discontinuity logging.
    pub fn set_effect_boundary_log(&self, enabled: bool) {
        let mut settings = self.buffer_settings.lock().unwrap();
        settings.effect_boundary_log = enabled;
    }

    /// Debug helper returning thread alive, state, and audio heard flags.
    pub fn debug_playback_state(&self) -> (bool, PlayerState, bool) {
        (
            self.playback_thread_exists.load(Ordering::SeqCst),
            *self.state.lock().unwrap(),
            self.audio_heard.load(Ordering::Relaxed),
        )
    }

    /// Debug helper indicating whether buffering has completed.
    pub fn debug_buffering_done(&self) -> bool {
        self.buffering_done.load(Ordering::Relaxed)
    }

    /// Debug helper returning internal timing markers in milliseconds.
    pub fn debug_timing_ms(&self) -> (u64, u64) {
        (
            self.last_chunk_ms.load(Ordering::Relaxed),
            self.last_time_update_ms.load(Ordering::Relaxed),
        )
    }

    /// Debug helper returning sink paused/empty flags and queued length.
    pub fn debug_sink_state(&self) -> (bool, bool, usize) {
        let sink = self.sink.lock().unwrap();
        let paused = sink.is_paused();
        let empty = sink.empty();
        let len = sink.len();
        (paused, empty, len)
    }
}
