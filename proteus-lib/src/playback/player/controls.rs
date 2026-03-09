//! Transport and lifecycle operations for `Player`.
//!
//! Methods here coordinate playback-state transitions with the runtime thread
//! and expose user-facing control primitives (play/pause/seek/stop, volume,
//! reporting hooks, and schedule inspection).

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use log::{debug, info, warn};

use super::{EndOfStreamAction, Player, PlayerState};
use crate::diagnostics::reporter::{Report, Reporter};

impl Player {
    /// Start playback from a specific timestamp (seconds).
    ///
    /// # Arguments
    ///
    /// * `ts` - Target start position in seconds.
    pub fn play_at(&mut self, ts: f64) {
        let trace_ms = current_ms();
        self.play_command_ms
            .store(trace_ms, std::sync::atomic::Ordering::Relaxed);
        debug!("play trace: play_at requested ts={:.3}", ts);
        let mut timestamp = self.ts.lock().unwrap();
        *timestamp = ts;
        drop(timestamp);

        self.request_effects_reset();
        self.clear_inline_effects_update();
        self.stop_and_join_playback_thread();
        debug!(
            "play trace: play_at after stop_and_join_playback_thread +{}ms",
            current_ms().saturating_sub(trace_ms)
        );
        self.initialize_thread(Some(ts));
        debug!(
            "play trace: play_at after initialize_thread +{}ms",
            current_ms().saturating_sub(trace_ms)
        );

        self.resume();
        debug!(
            "play trace: play_at after resume() request +{}ms",
            current_ms().saturating_sub(trace_ms)
        );

        self.wait_for_audio_heard(Duration::from_secs(5));
    }

    /// Start playback from the current timestamp.
    ///
    /// If no playback thread is currently alive, a new runtime is created.
    pub fn play(&mut self) {
        let trace_ms = current_ms();
        self.play_command_ms
            .store(trace_ms, std::sync::atomic::Ordering::Relaxed);
        info!("Playing audio");
        let thread_exists = self
            .playback_thread_exists
            .load(std::sync::atomic::Ordering::SeqCst);
        debug!(
            "play trace: play requested thread_exists={} state={:?}",
            thread_exists,
            *self.state.lock().unwrap()
        );

        if !thread_exists {
            self.initialize_thread(None);
            debug!(
                "play trace: play after initialize_thread +{}ms",
                current_ms().saturating_sub(trace_ms)
            );
        }

        self.resume();
        debug!(
            "play trace: play after resume() request +{}ms",
            current_ms().saturating_sub(trace_ms)
        );

        self.wait_for_audio_heard(Duration::from_secs(5));
    }

    /// Pause playback.
    pub fn pause(&self) {
        self.state.lock().unwrap().clone_from(&PlayerState::Pausing);
    }

    /// Resume playback if paused.
    pub fn resume(&self) {
        let trace_ms = self
            .play_command_ms
            .load(std::sync::atomic::Ordering::Relaxed);
        if trace_ms > 0 {
            debug!(
                "play trace: resume requested +{}ms",
                current_ms().saturating_sub(trace_ms)
            );
        } else {
            debug!("play trace: resume requested");
        }
        self.state
            .lock()
            .unwrap()
            .clone_from(&PlayerState::Resuming);
    }

    /// Stop the current playback thread and wait for it to exit.
    ///
    /// Internal state is moved through `Stopping` and finalized as `Stopped`.
    pub fn stop_and_join_playback_thread(&self) {
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
        self.join_playback_thread();

        self.state.lock().unwrap().clone_from(&PlayerState::Stopped);
    }

    /// Join the current playback thread handle if one is present.
    pub(in crate::playback::player) fn join_playback_thread(&self) {
        if let Some(handle) = self.playback_thread_handle.lock().unwrap().take() {
            if handle.join().is_err() {
                warn!("playback thread panicked during join");
            }
        }
    }

    /// Stop playback and reset timing state.
    pub fn stop(&self) {
        self.stop_and_join_playback_thread();
        self.ts.lock().unwrap().clone_from(&0.0);
    }

    /// Set the action applied automatically when playback reaches the end.
    ///
    /// # Arguments
    ///
    /// * `action` - End-of-stream behavior (`Stop` or `Pause`).
    pub fn set_end_of_stream_action(&self, action: EndOfStreamAction) {
        *self.end_of_stream_action.lock().unwrap() = action;
    }

    /// Get the current end-of-stream action.
    pub fn get_end_of_stream_action(&self) -> EndOfStreamAction {
        *self.end_of_stream_action.lock().unwrap()
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

        let state = *self.state.lock().unwrap();
        let was_active = seek_should_resume(state);
        let (seek_fade_out_ms, seek_fade_in_ms) = {
            let settings = self.buffer_settings.lock().unwrap();
            (settings.seek_fade_out_ms, settings.seek_fade_in_ms)
        };
        if was_active && seek_fade_out_ms > 0.0 {
            self.fade_current_sink_out(seek_fade_out_ms);
        }
        self.request_effects_reset();
        self.clear_inline_effects_update();

        self.stop_and_join_playback_thread();
        self.initialize_thread(Some(ts));
        if was_active {
            *self.next_resume_fade_ms.lock().unwrap() = Some(seek_fade_in_ms);
            self.resume();
        } else {
            self.state.lock().unwrap().clone_from(&state);
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
        let trace_ms = self
            .play_command_ms
            .load(std::sync::atomic::Ordering::Relaxed);
        if trace_ms > 0 {
            debug!(
                "play trace: wait_for_audio_heard start timeout_ms={} +{}ms",
                timeout.as_millis(),
                current_ms().saturating_sub(trace_ms)
            );
        }
        let start = Instant::now();
        loop {
            if self.audio_heard.load(std::sync::atomic::Ordering::Relaxed) {
                if trace_ms > 0 {
                    debug!(
                        "play trace: audio_heard observed +{}ms (waited {}ms)",
                        current_ms().saturating_sub(trace_ms),
                        start.elapsed().as_millis()
                    );
                }
                return true;
            }
            if self.thread_finished() {
                warn!("playback thread ended before audio was heard");
                return false;
            }
            if start.elapsed() >= timeout {
                warn!("timed out waiting for audio to start");
                if trace_ms > 0 {
                    warn!(
                        "play trace: wait_for_audio_heard timeout +{}ms",
                        current_ms().saturating_sub(trace_ms)
                    );
                }
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
    /// Each entry is `(time_seconds, grouped_selected_ids_or_paths)`, where the
    /// inner groups map to logical tracks and contain all selections for each
    /// track (for example when `selections_count > 1`).
    pub fn get_shuffle_schedule(&self) -> Vec<(f64, Vec<Vec<String>>)> {
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
        if let Some(reporter) = self.reporter.as_ref() {
            reporter.lock().unwrap().stop();
        }

        let reporter = Arc::new(Mutex::new(Reporter::new(
            self.ts.clone(),
            self.volume.clone(),
            self.duration.clone(),
            self.state.clone(),
            reporting,
            reporting_interval,
        )));

        reporter.lock().unwrap().start();

        self.reporter = Some(reporter);
    }
}

fn seek_should_resume(state: PlayerState) -> bool {
    matches!(state, PlayerState::Playing | PlayerState::Resuming)
}

fn current_ms() -> u64 {
    use std::time::SystemTime;

    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{current_ms, seek_should_resume, EndOfStreamAction, Player, PlayerState};
    use crate::container::prot::PathsTrack;
    use std::sync::atomic::Ordering;

    #[test]
    fn current_ms_returns_non_zero_epoch_time() {
        assert!(current_ms() > 0);
    }

    #[test]
    fn seek_should_resume_only_for_active_states() {
        assert!(seek_should_resume(PlayerState::Playing));
        assert!(seek_should_resume(PlayerState::Resuming));
        assert!(!seek_should_resume(PlayerState::Paused));
        assert!(!seek_should_resume(PlayerState::Stopped));
        assert!(!seek_should_resume(PlayerState::Stopping));
        assert!(!seek_should_resume(PlayerState::Pausing));
    }

    #[test]
    fn pause_and_resume_update_player_state() {
        let player = lifecycle_test_player();
        player.pause();
        assert_eq!(*player.state.lock().unwrap(), PlayerState::Pausing);
        player.resume();
        assert_eq!(*player.state.lock().unwrap(), PlayerState::Resuming);
    }

    #[test]
    fn stop_resets_timestamp_and_marks_stopped_when_thread_already_finished() {
        let player = lifecycle_test_player();
        *player.ts.lock().unwrap() = 12.5;
        player.stop();
        assert_eq!(player.get_time(), 0.0);
        assert_eq!(*player.state.lock().unwrap(), PlayerState::Stopped);
        assert!(player.abort.load(Ordering::SeqCst));
    }

    #[test]
    fn end_of_stream_action_round_trip() {
        let player = lifecycle_test_player();
        player.set_end_of_stream_action(EndOfStreamAction::Pause);
        assert_eq!(player.get_end_of_stream_action(), EndOfStreamAction::Pause);
        player.set_end_of_stream_action(EndOfStreamAction::Stop);
        assert_eq!(player.get_end_of_stream_action(), EndOfStreamAction::Stop);
    }

    fn lifecycle_test_player() -> Player {
        let mut player = Player::new_from_file_paths(vec![PathsTrack::new_from_file_paths(vec![
            "/tmp/nonexistent.wav".to_string(),
        ])]);
        // Avoid waiting for runtime thread work in unit tests.
        player.playback_thread_exists.store(false, Ordering::SeqCst);
        player.abort.store(true, Ordering::SeqCst);
        *player.playback_thread_handle.lock().unwrap() = None;
        *player.state.lock().unwrap() = PlayerState::Stopped;
        player.reporter = None;
        player
    }
}
