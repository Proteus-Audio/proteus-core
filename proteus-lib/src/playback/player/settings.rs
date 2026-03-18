//! Runtime tuning and debug accessors for `Player`.
//!
//! These methods expose buffering/fade/jitter controls used by the runtime
//! worker thread, plus lightweight debug snapshots for diagnostics.

use std::sync::atomic::Ordering;

use crate::playback::engine::{InlineTrackMixUpdate, PlaybackBufferSettings};

use super::{Player, PlayerState};

fn clamp_non_negative(value: f32) -> f32 {
    value.max(0.0)
}

impl Player {
    /// Apply the opt-in live-authoring profile without changing library defaults.
    ///
    /// This is intended for editor-style workflows where bounded control
    /// latency matters more than maximum buffering headroom. Player apps should
    /// continue using their existing stable playback settings unless they
    /// explicitly want this tradeoff.
    pub fn configure_for_live_authoring(&self) {
        self.set_buffer_settings(PlaybackBufferSettings::live_authoring());
    }

    /// Apply a cohesive in-place update to buffer settings under one lock.
    ///
    /// # Arguments
    ///
    /// * `update` - Closure that mutates [`PlaybackBufferSettings`].
    pub fn update_buffer_settings<F>(&self, update: F)
    where
        F: FnOnce(&mut PlaybackBufferSettings),
    {
        let mut settings = self.lock_buffer_settings_recoverable();
        update(&mut settings);
    }

    /// Replace all playback buffer settings in one operation.
    ///
    /// # Arguments
    ///
    /// * `settings` - Full playback buffer settings snapshot to apply.
    pub fn set_buffer_settings(&self, settings: PlaybackBufferSettings) {
        self.update_buffer_settings(|current| *current = settings);
    }

    /// Configure the minimum buffered audio (ms) before playback starts.
    ///
    /// # Arguments
    ///
    /// * `start_buffer_ms` - Startup prebuffer target in milliseconds.
    pub fn set_start_buffer_ms(&self, start_buffer_ms: f32) {
        self.update_buffer_settings(|settings| {
            settings.start_buffer_ms = clamp_non_negative(start_buffer_ms);
        });
    }

    /// Configure heuristic end-of-track threshold for containers (ms).
    ///
    /// # Arguments
    ///
    /// * `track_eos_ms` - End-of-track threshold in milliseconds.
    pub fn set_track_eos_ms(&self, track_eos_ms: f32) {
        self.update_buffer_settings(|settings| {
            settings.track_eos_ms = clamp_non_negative(track_eos_ms);
        });
    }

    /// Configure minimum sink chunks queued before playback starts/resumes.
    pub fn set_start_sink_chunks(&self, chunks: usize) {
        self.update_buffer_settings(|settings| {
            settings.start_sink_chunks = chunks;
        });
    }

    /// Configure the maximum sink chunks queued before producer backpressure.
    ///
    /// Set to `0` to disable this guard.
    pub fn set_max_sink_chunks(&self, chunks: usize) {
        self.update_buffer_settings(|settings| {
            settings.max_sink_chunks = chunks;
        });
    }

    /// Configure the startup silence pre-roll (ms).
    pub fn set_startup_silence_ms(&self, ms: f32) {
        self.update_buffer_settings(|settings| {
            settings.startup_silence_ms = clamp_non_negative(ms);
        });
    }

    /// Configure the startup fade-in length (ms).
    pub fn set_startup_fade_ms(&self, ms: f32) {
        self.update_buffer_settings(|settings| {
            settings.startup_fade_ms = clamp_non_negative(ms);
        });
    }

    /// Configure seek fade-out length (ms) before restarting playback.
    pub fn set_seek_fade_out_ms(&self, ms: f32) {
        self.update_buffer_settings(|settings| {
            settings.seek_fade_out_ms = clamp_non_negative(ms);
        });
    }

    /// Configure seek fade-in length (ms) after restarting playback.
    pub fn set_seek_fade_in_ms(&self, ms: f32) {
        self.update_buffer_settings(|settings| {
            settings.seek_fade_in_ms = clamp_non_negative(ms);
        });
    }

    /// Configure the append jitter logging threshold (ms). 0 disables logging.
    pub fn set_append_jitter_log_ms(&self, ms: f32) {
        self.update_buffer_settings(|settings| {
            settings.append_jitter_log_ms = clamp_non_negative(ms);
        });
    }

    /// Configure inline effects transition duration (ms) for `set_effects_inline`.
    pub fn set_inline_effects_transition_ms(&self, ms: f32) {
        self.update_buffer_settings(|settings| {
            settings.inline_effects_transition_ms = clamp_non_negative(ms);
        });
    }

    /// Enable or disable per-effect boundary discontinuity logging.
    pub fn set_effect_boundary_log(&self, enabled: bool) {
        self.update_buffer_settings(|settings| {
            settings.effect_boundary_log = enabled;
        });
    }

    /// Configure the duration (ms) used for per-parameter smoothing ramps.
    ///
    /// This controls how quickly individual effect parameter changes (gain,
    /// filter cutoff, etc.) are ramped to their new values. The default is
    /// 5.0 ms. A value of 0.0 disables smoothing (parameters snap instantly).
    pub fn set_parameter_ramp_ms(&self, ms: f32) {
        self.update_buffer_settings(|settings| {
            settings.parameter_ramp_ms = clamp_non_negative(ms);
        });
    }

    /// Configure the time-based queued-output limit for the sink.
    ///
    /// When set, the playback worker blocks the producer once queued output
    /// exceeds this budget. Set to `None` to disable. This is orthogonal to
    /// the chunk-count limit (`set_max_sink_chunks`); when both are active
    /// the stricter effective cap wins.
    ///
    /// # Arguments
    ///
    /// * `ms` - Maximum queued output in milliseconds, or `None` to disable.
    pub fn set_max_sink_latency_ms(&self, ms: Option<f32>) {
        self.update_buffer_settings(|settings| {
            settings.max_sink_latency_ms = ms.map(|v| v.max(0.0));
        });
    }

    /// Configure the target output slice duration for sink appends.
    ///
    /// When set, post-DSP output is sliced into chunks of approximately this
    /// duration before being sent to the worker thread. This decouples
    /// internal DSP batch size from sink append granularity, which is
    /// important for convolution-heavy chains in authoring mode. Set to
    /// `None` to disable (full batches are sent as single chunks).
    ///
    /// # Arguments
    ///
    /// * `ms` - Target slice duration in milliseconds, or `None` to disable.
    pub fn set_output_slice_ms(&self, ms: Option<f32>) {
        self.update_buffer_settings(|settings| {
            settings.output_slice_ms = ms.map(|v| v.max(0.0));
        });
    }

    /// Update per-slot track level/pan without restarting playback.
    ///
    /// This mutates the underlying track model and queues an inline update for
    /// the active mix thread. Returns `false` if `slot_index` is out of range.
    pub fn set_track_mix_inline(&self, slot_index: usize, level: f32, pan: f32) -> bool {
        let linked_slots = {
            let mut prot = self.lock_prot_invariant();
            if !prot.set_slot_mix_settings(slot_index, level, pan) {
                return false;
            }
            prot.linked_slot_indices(slot_index)
        };
        let Some(linked_slots) = linked_slots else {
            return false;
        };

        let mut pending = self.lock_inline_track_mix_updates_recoverable();
        for slot_index in linked_slots {
            pending.push(InlineTrackMixUpdate {
                slot_index,
                level,
                pan,
            });
        }
        true
    }

    /// Debug helper returning thread alive, state, and audio heard flags.
    ///
    /// Both `playback_thread_exists` and `audio_heard` use `Acquire` to
    /// synchronize-with the corresponding `Release` stores on the worker thread.
    pub fn debug_playback_state(&self) -> (bool, PlayerState, bool) {
        (
            self.playback_thread_exists.load(Ordering::Acquire),
            *self.lock_state_invariant(),
            self.audio_heard.load(Ordering::Acquire),
        )
    }

    /// Debug helper indicating whether buffering has completed.
    ///
    /// Uses `Acquire` to synchronize-with the `Release` store in
    /// `mark_buffering_complete`, so the completion is fully visible once
    /// this returns `true`.
    pub fn debug_buffering_done(&self) -> bool {
        self.buffering_done.load(Ordering::Acquire)
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
        let sink = self.lock_sink_recoverable();
        let paused = sink.is_paused();
        let empty = sink.empty();
        let len = sink.len();
        (paused, empty, len)
    }
}

#[cfg(test)]
mod tests {
    use super::clamp_non_negative;
    use crate::container::prot::PathsTrack;
    use crate::playback::player::{Player, PlayerState};
    use std::sync::atomic::Ordering;

    #[test]
    fn clamp_non_negative_zeroes_negative_values() {
        assert_eq!(clamp_non_negative(-12.5), 0.0);
    }

    #[test]
    fn clamp_non_negative_keeps_positive_values() {
        assert_eq!(clamp_non_negative(12.5), 12.5);
    }

    #[test]
    fn set_parameter_ramp_ms_updates_buffer_settings() {
        let player = test_player();
        player.set_parameter_ramp_ms(12.5);
        assert_eq!(
            player.lock_buffer_settings_recoverable().parameter_ramp_ms,
            12.5
        );
    }

    #[test]
    fn configure_for_live_authoring_applies_opt_in_profile() {
        let player = test_player();
        player.configure_for_live_authoring();
        let settings = *player.lock_buffer_settings_recoverable();
        assert_eq!(settings.start_buffer_ms, 20.0);
        assert_eq!(settings.start_sink_chunks, 1);
        assert_eq!(settings.max_sink_chunks, 2);
        assert_eq!(settings.startup_fade_ms, 80.0);
        assert_eq!(settings.seek_fade_out_ms, 20.0);
        assert_eq!(settings.seek_fade_in_ms, 50.0);
        assert_eq!(settings.inline_effects_transition_ms, 15.0);
        assert_eq!(settings.parameter_ramp_ms, 5.0);
        assert_eq!(settings.max_sink_latency_ms, Some(60.0));
        assert_eq!(settings.output_slice_ms, Some(30.0));
    }

    #[test]
    fn set_max_sink_latency_ms_updates_buffer_settings() {
        let player = test_player();
        player.set_max_sink_latency_ms(Some(80.0));
        assert_eq!(
            player
                .lock_buffer_settings_recoverable()
                .max_sink_latency_ms,
            Some(80.0)
        );
    }

    #[test]
    fn set_max_sink_latency_ms_none_disables() {
        let player = test_player();
        player.set_max_sink_latency_ms(Some(50.0));
        player.set_max_sink_latency_ms(None);
        assert!(player
            .lock_buffer_settings_recoverable()
            .max_sink_latency_ms
            .is_none());
    }

    #[test]
    fn set_max_sink_latency_ms_clamps_negative() {
        let player = test_player();
        player.set_max_sink_latency_ms(Some(-10.0));
        assert_eq!(
            player
                .lock_buffer_settings_recoverable()
                .max_sink_latency_ms,
            Some(0.0)
        );
    }

    #[test]
    fn set_output_slice_ms_updates_buffer_settings() {
        let player = test_player();
        player.set_output_slice_ms(Some(30.0));
        assert_eq!(
            player.lock_buffer_settings_recoverable().output_slice_ms,
            Some(30.0)
        );
    }

    #[test]
    fn set_output_slice_ms_none_disables() {
        let player = test_player();
        player.set_output_slice_ms(Some(30.0));
        player.set_output_slice_ms(None);
        assert!(player
            .lock_buffer_settings_recoverable()
            .output_slice_ms
            .is_none());
    }

    fn test_player() -> Player {
        let player = Player::new_from_file_paths(vec![PathsTrack::new_from_file_paths(vec![
            "/tmp/nonexistent.wav".to_string(),
        ])]);
        player.playback_thread_exists.store(false, Ordering::SeqCst);
        player.abort.store(true, Ordering::SeqCst);
        *player.lock_playback_thread_handle_invariant() = None;
        *player.lock_state_invariant() = PlayerState::Stopped;
        player
    }
}
