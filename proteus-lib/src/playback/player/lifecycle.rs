//! Playback-thread lifecycle helpers.

use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use log::{debug, warn};

use super::{Player, PlayerState};
use crate::playback::engine::DspChainMetrics;

impl Player {
    /// Stop the current playback thread and wait for it to exit.
    ///
    /// Internal state is moved through `Stopping` and finalized as `Stopped`.
    pub fn stop_and_join_playback_thread(&self) {
        self.lock_state_invariant()
            .clone_from(&PlayerState::Stopping);
        {
            let sink = self.lock_sink_recoverable();
            sink.stop();
        }
        self.abort.store(true, Ordering::SeqCst);
        self.worker_notify.notify();

        while !self.thread_finished() {
            thread::sleep(Duration::from_millis(10));
        }
        self.join_playback_thread();

        self.lock_state_invariant()
            .clone_from(&PlayerState::Stopped);
    }

    /// Join the current playback thread handle if one is present.
    pub(in crate::playback::player) fn join_playback_thread(&self) {
        if let Some(handle) = self.lock_playback_thread_handle_invariant().take() {
            if handle.join().is_err() {
                warn!("playback thread panicked during join");
            }
        }
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
        let trace_ms = self.play_command_ms.load(Ordering::Relaxed);
        if trace_ms > 0 {
            debug!(
                "play trace: wait_for_audio_heard start timeout_ms={} +{}ms",
                timeout.as_millis(),
                current_ms().saturating_sub(trace_ms)
            );
        }
        let start = Instant::now();
        loop {
            // Acquire: synchronize-with the Release store in update_sink so that
            // any sink state written before audio_heard was set is visible here.
            if self.audio_heard.load(Ordering::Acquire) {
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
}

pub(super) fn current_ms() -> u64 {
    use std::time::SystemTime;

    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub(super) fn drop_cleanup(player: &mut Player) {
    if player.handle_count.fetch_sub(1, Ordering::AcqRel) != 1 {
        return;
    }
    if player.shutdown_once.swap(true, Ordering::AcqRel) {
        return;
    }

    if let Some(reporter) = player.reporter.take() {
        Player::lock_reporter_invariant(&reporter).stop();
    }

    if player.playback_thread_exists.load(Ordering::Acquire) {
        player.stop_and_join_playback_thread();
    } else {
        player.abort.store(true, Ordering::SeqCst);
        player.join_playback_thread();
    }

    {
        let sink = player.lock_sink_recoverable();
        sink.stop();
        sink.clear();
    }

    {
        let mut finished_tracks = player.lock_finished_tracks_recoverable();
        finished_tracks.clear();
        finished_tracks.shrink_to_fit();
    }

    {
        let mut effects = player.lock_effects_recoverable();
        effects.clear();
        effects.shrink_to_fit();
    }

    {
        let mut inline_effects_update = player.lock_inline_effects_update_recoverable();
        *inline_effects_update = None;
    }

    {
        let mut inline_track_mix_updates = player.lock_inline_track_mix_updates_recoverable();
        inline_track_mix_updates.clear();
        inline_track_mix_updates.shrink_to_fit();
    }

    {
        let mut dsp_metrics = player.lock_dsp_metrics_recoverable();
        *dsp_metrics = DspChainMetrics::default();
    }

    {
        let mut output_meter = player.lock_output_meter_recoverable();
        output_meter.reset();
    }

    debug!("player dropped");

    *player.lock_duration_recoverable() = 0.0;
    *player.lock_ts_recoverable() = 0.0;
    *player.lock_next_resume_fade_ms_recoverable() = None;
    player.buffering_done.store(false, Ordering::Relaxed);
    player.last_chunk_ms.store(0, Ordering::Relaxed);
    player.last_time_update_ms.store(0, Ordering::Relaxed);
    player.audio_heard.store(false, Ordering::Relaxed);
    player.impulse_response_override = None;
    player.impulse_response_tail_override = None;
}
