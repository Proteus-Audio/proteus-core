//! Transport and lifecycle operations for `Player`.
//!
//! Methods here coordinate playback-state transitions with the runtime thread
//! and expose user-facing control primitives (play/pause/seek/stop, volume,
//! reporting hooks, and schedule inspection).

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use log::{info, warn};

use crate::diagnostics::reporter::{Report, Reporter};

use super::{Player, PlayerState};

impl Player {
    /// Start playback from a specific timestamp (seconds).
    ///
    /// # Arguments
    ///
    /// * `ts` - Target start position in seconds.
    pub fn play_at(&mut self, ts: f64) {
        let mut timestamp = self.ts.lock().unwrap();
        *timestamp = ts;
        drop(timestamp);

        self.request_effects_reset();
        self.clear_inline_effects_update();
        self.kill_current();
        self.initialize_thread(Some(ts));

        self.resume();

        self.wait_for_audio_heard(Duration::from_secs(5));
    }

    /// Start playback from the current timestamp.
    ///
    /// If no playback thread is currently alive, a new runtime is created.
    pub fn play(&mut self) {
        info!("Playing audio");
        let thread_exists = self
            .playback_thread_exists
            .load(std::sync::atomic::Ordering::SeqCst);

        if !thread_exists {
            self.initialize_thread(None);
        }

        self.resume();

        self.wait_for_audio_heard(Duration::from_secs(5));
    }

    /// Pause playback.
    pub fn pause(&self) {
        self.state.lock().unwrap().clone_from(&PlayerState::Pausing);
    }

    /// Resume playback if paused.
    pub fn resume(&self) {
        self.state
            .lock()
            .unwrap()
            .clone_from(&PlayerState::Resuming);
    }

    /// Stop the current playback thread and wait for it to exit.
    ///
    /// Internal state is moved through `Stopping` and finalized as `Stopped`.
    pub fn kill_current(&self) {
        self.state
            .lock()
            .unwrap()
            .clone_from(&PlayerState::Stopping);
        {
            let sink = self.sink.lock().unwrap();
            sink.stop();
        }
        self.abort.store(true, std::sync::atomic::Ordering::SeqCst);

        while !self.thread_finished() {
            thread::sleep(Duration::from_millis(10));
        }

        self.state.lock().unwrap().clone_from(&PlayerState::Stopped);
    }

    /// Stop playback and reset timing state.
    pub fn stop(&self) {
        self.kill_current();
        self.ts.lock().unwrap().clone_from(&0.0);
    }

    /// Return true if playback is currently active.
    pub fn is_playing(&self) -> bool {
        let state = self.state.lock().unwrap();
        *state == PlayerState::Playing
    }

    /// Return true if playback is currently paused.
    pub fn is_paused(&self) -> bool {
        let state = self.state.lock().unwrap();
        *state == PlayerState::Paused
    }

    /// Get the current playback time in seconds.
    pub fn get_time(&self) -> f64 {
        let ts = self.ts.lock().unwrap();
        *ts
    }

    /// Return `true` when no playback worker thread is alive.
    pub(super) fn thread_finished(&self) -> bool {
        let playback_thread_exists = self
            .playback_thread_exists
            .load(std::sync::atomic::Ordering::SeqCst);
        !playback_thread_exists
    }

    /// Return true if playback has reached the end.
    pub fn is_finished(&self) -> bool {
        self.thread_finished()
    }

    /// Block the current thread until playback finishes.
    pub fn sleep_until_end(&self) {
        loop {
            if self.thread_finished() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// Get the total duration (seconds) of the active selection.
    pub fn get_duration(&self) -> f64 {
        let duration = self.duration.lock().unwrap();
        *duration
    }

    /// Seek to the given timestamp (seconds).
    ///
    /// Seeking rebuilds the playback runtime at `ts` and applies configured
    /// seek fade-out/fade-in behavior when currently playing.
    ///
    /// # Arguments
    ///
    /// * `ts` - New playback position in seconds.
    pub fn seek(&mut self, ts: f64) {
        let mut timestamp = self.ts.lock().unwrap();
        *timestamp = ts;
        drop(timestamp);

        let state = self.state.lock().unwrap().clone();
        let (seek_fade_out_ms, seek_fade_in_ms) = {
            let settings = self.buffer_settings.lock().unwrap();
            (settings.seek_fade_out_ms, settings.seek_fade_in_ms)
        };
        if matches!(state, PlayerState::Playing | PlayerState::Resuming) && seek_fade_out_ms > 0.0 {
            self.fade_current_sink_out(seek_fade_out_ms);
        }
        self.request_effects_reset();
        self.clear_inline_effects_update();

        self.kill_current();
        self.state.lock().unwrap().clone_from(&state);
        self.initialize_thread(Some(ts));

        if matches!(state, PlayerState::Playing | PlayerState::Resuming) {
            *self.next_resume_fade_ms.lock().unwrap() = Some(seek_fade_in_ms);
            self.resume();
        }
    }

    /// Apply a short linear fade-out to the current sink before disruptive ops.
    ///
    /// # Arguments
    ///
    /// * `fade_ms` - Fade duration in milliseconds.
    fn fade_current_sink_out(&self, fade_ms: f32) {
        let steps = ((fade_ms / 5.0).ceil() as u32).max(1);
        let step_ms = (fade_ms / steps as f32).max(1.0) as u64;
        let sink = self.sink.lock().unwrap();
        let start_volume = sink.volume().max(0.0);
        if start_volume <= 0.0 {
            return;
        }
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let gain = start_volume * (1.0 - t);
            sink.set_volume(gain.max(0.0));
            thread::sleep(Duration::from_millis(step_ms));
        }
    }

    /// Refresh active track selections from the underlying container.
    ///
    /// Existing reverb overrides are re-applied and active playback is
    /// restarted at the current timestamp.
    pub fn refresh_tracks(&mut self) {
        let mut prot = self.prot.lock().unwrap();
        prot.refresh_tracks();
        if let Some(spec) = self.impulse_response_override.clone() {
            prot.set_impulse_response_spec(spec);
        }
        if let Some(tail_db) = self.impulse_response_tail_override {
            prot.set_impulse_response_tail_db(tail_db);
        }
        drop(prot);

        self.request_effects_reset();
        self.clear_inline_effects_update();
        if self.thread_finished() {
            return;
        }

        let ts = self.get_time();
        self.seek(ts);

        if self.is_playing() {
            self.resume();
        }

        self.wait_for_audio_heard(Duration::from_secs(5));
    }

    /// Wait until the runtime reports that at least one chunk was appended.
    ///
    /// # Arguments
    ///
    /// * `timeout` - Maximum wait duration before returning `false`.
    ///
    /// # Returns
    ///
    /// `true` once audio has been observed, `false` on timeout or early thread
    /// termination.
    pub(super) fn wait_for_audio_heard(&self, timeout: Duration) -> bool {
        let start = Instant::now();
        loop {
            if self.audio_heard.load(std::sync::atomic::Ordering::Relaxed) {
                return true;
            }
            if self.thread_finished() {
                warn!("playback thread ended before audio was heard");
                return false;
            }
            if start.elapsed() >= timeout {
                warn!("timed out waiting for audio to start");
                return false;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Shuffle track selections and restart playback.
    pub fn shuffle(&mut self) {
        self.refresh_tracks();
    }

    /// Set the playback volume (linear gain).
    ///
    /// # Arguments
    ///
    /// * `new_volume` - Desired sink gain multiplier.
    pub fn set_volume(&mut self, new_volume: f32) {
        let sink = self.sink.lock().unwrap();
        sink.set_volume(new_volume);
        drop(sink);

        let mut volume = self.volume.lock().unwrap();
        *volume = new_volume;
        drop(volume);
    }

    /// Get the current playback volume.
    pub fn get_volume(&self) -> f32 {
        *self.volume.lock().unwrap()
    }

    /// Get the track identifiers used for display.
    pub fn get_ids(&self) -> Vec<String> {
        let prot = self.prot.lock().unwrap();
        prot.get_ids()
    }

    /// Get the full timestamped shuffle schedule used by playback.
    ///
    /// Each entry is `(time_seconds, selected_ids_or_paths)`.
    pub fn get_shuffle_schedule(&self) -> Vec<(f64, Vec<String>)> {
        let prot = self.prot.lock().unwrap();
        prot.get_shuffle_schedule()
    }

    /// Enable periodic reporting of playback status for UI consumers.
    ///
    /// Any previous reporter instance is stopped before a new one is started.
    ///
    /// # Arguments
    ///
    /// * `reporting` - Callback invoked with periodic playback snapshots.
    /// * `reporting_interval` - Time between callback invocations.
    pub fn set_reporting(
        &mut self,
        reporting: Arc<Mutex<dyn Fn(Report) + Send>>,
        reporting_interval: Duration,
    ) {
        if self.reporter.is_some() {
            self.reporter.as_ref().unwrap().lock().unwrap().stop();
        }

        let reporter = Arc::new(Mutex::new(Reporter::new(
            Arc::new(Mutex::new(self.clone())),
            reporting,
            reporting_interval,
        )));

        reporter.lock().unwrap().start();

        self.reporter = Some(reporter);
    }
}
