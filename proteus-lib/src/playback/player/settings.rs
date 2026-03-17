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
    /// Apply a cohesive in-place update to buffer settings under one lock.
    ///
    /// # Arguments
    ///
    /// * `update` - Closure that mutates [`PlaybackBufferSettings`].
    pub fn update_buffer_settings<F>(&self, update: F)
    where
        F: FnOnce(&mut PlaybackBufferSettings),
    {
        let mut settings = self.buffer_settings.lock().unwrap_or_else(|_| {
            panic!("buffer settings lock poisoned — a thread panicked while holding it")
        });
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

    /// Update per-slot track level/pan without restarting playback.
    ///
    /// This mutates the underlying track model and queues an inline update for
    /// the active mix thread. Returns `false` if `slot_index` is out of range.
    pub fn set_track_mix_inline(&self, slot_index: usize, level: f32, pan: f32) -> bool {
        let linked_slots = {
            let mut prot = self.prot.lock().unwrap_or_else(|_| {
                panic!("prot lock poisoned — a thread panicked while holding it")
            });
            if !prot.set_slot_mix_settings(slot_index, level, pan) {
                return false;
            }
            prot.linked_slot_indices(slot_index)
        };
        let Some(linked_slots) = linked_slots else {
            return false;
        };

        let mut pending = self.inline_track_mix_updates.lock().unwrap_or_else(|_| {
            panic!("inline track mix updates lock poisoned — a thread panicked while holding it")
        });
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
            *self.state.lock().unwrap_or_else(|_| {
                panic!("state lock poisoned — a thread panicked while holding it")
            }),
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
        let sink = self
            .sink
            .lock()
            .unwrap_or_else(|_| panic!("sink lock poisoned — a thread panicked while holding it"));
        let paused = sink.is_paused();
        let empty = sink.empty();
        let len = sink.len();
        (paused, empty, len)
    }
}

#[cfg(test)]
mod tests {
    use super::clamp_non_negative;

    #[test]
    fn clamp_non_negative_zeroes_negative_values() {
        assert_eq!(clamp_non_negative(-12.5), 0.0);
    }

    #[test]
    fn clamp_non_negative_keeps_positive_values() {
        assert_eq!(clamp_non_negative(12.5), 12.5);
    }
}
