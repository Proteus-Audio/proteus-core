//! Read-only player state helpers.

use std::thread;
use std::time::Duration;

use super::{Player, PlayerState};

impl Player {
    /// Get read-only metadata describing the active container or file list.
    pub fn audio_info(&self) -> &crate::container::info::Info {
        &self.info
    }

    /// Return true if playback is currently active.
    pub fn is_playing(&self) -> bool {
        *self
            .state
            .lock()
            .unwrap_or_else(|_| panic!("state lock poisoned — a thread panicked while holding it"))
            == PlayerState::Playing
    }

    /// Return true if playback is currently paused.
    pub fn is_paused(&self) -> bool {
        *self
            .state
            .lock()
            .unwrap_or_else(|_| panic!("state lock poisoned — a thread panicked while holding it"))
            == PlayerState::Paused
    }

    /// Get the current playback time in seconds.
    pub fn playback_position_secs(&self) -> f64 {
        *self
            .ts
            .lock()
            .unwrap_or_else(|_| panic!("ts lock poisoned — a thread panicked while holding it"))
    }

    /// Get the current playback time in seconds.
    pub fn get_time(&self) -> f64 {
        self.playback_position_secs()
    }

    /// Get the finished track identifiers as a detached snapshot.
    pub fn finished_track_indices(&self) -> Vec<i32> {
        self.finished_tracks
            .lock()
            .unwrap_or_else(|_| {
                panic!("finished tracks lock poisoned — a thread panicked while holding it")
            })
            .clone()
    }

    /// Return `true` when no playback worker thread is alive.
    pub(super) fn thread_finished(&self) -> bool {
        !self
            .playback_thread_exists
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Return `true` when no playback worker thread is alive.
    ///
    /// This reflects runtime lifecycle state, not end-of-stream semantics.
    /// Use playback state and timestamp/duration checks for stricter EOS logic.
    pub fn is_finished(&self) -> bool {
        self.thread_finished()
    }

    /// Block the current thread until playback finishes.
    pub fn sleep_until_end(&self) {
        while !self.thread_finished() {
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// Get the total duration (seconds) of the active selection.
    pub fn get_duration(&self) -> f64 {
        *self.duration.lock().unwrap_or_else(|_| {
            panic!("duration lock poisoned — a thread panicked while holding it")
        })
    }

    /// Get the track identifiers used for display.
    pub fn get_ids(&self) -> Vec<String> {
        self.prot
            .lock()
            .unwrap_or_else(|_| panic!("prot lock poisoned — a thread panicked while holding it"))
            .get_ids()
    }

    /// Get the full timestamped shuffle schedule used by playback.
    ///
    /// Each entry is `(time_seconds, grouped_selected_ids_or_paths)`, where the
    /// inner groups map to logical tracks and contain all selections for each
    /// track (for example when `selections_count > 1`).
    pub fn get_shuffle_schedule(&self) -> Vec<(f64, Vec<Vec<String>>)> {
        self.prot
            .lock()
            .unwrap_or_else(|_| panic!("prot lock poisoned — a thread panicked while holding it"))
            .get_shuffle_schedule()
    }
}
