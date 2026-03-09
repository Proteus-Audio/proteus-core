//! Playback-thread bootstrap for `Player`.
//!
//! This module prepares shared state, resets per-run counters, and spawns the
//! worker loop that performs decoding handoff and sink append operations.

use log::debug;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use super::super::Player;
use super::now_ms;
use super::worker::{open_output_stream_with_retry, run_playback_thread, ThreadContext};

impl Player {
    /// Initialize and spawn a fresh playback thread.
    ///
    /// # Arguments
    ///
    /// * `ts` - Optional starting time in seconds. When `None`, playback starts
    ///   from `0.0`.
    pub(in crate::playback::player) fn initialize_thread(&mut self, ts: Option<f64>) {
        let trace_ms = self.play_command_ms.load(Ordering::Relaxed);
        let now = now_ms();
        if trace_ms > 0 {
            debug!(
                "play trace: initialize_thread start ts={:?} +{}ms",
                ts,
                now.saturating_sub(trace_ms)
            );
        } else {
            debug!("play trace: initialize_thread start ts={:?}", ts);
        }
        self.join_playback_thread();

        let mut finished_tracks = self.finished_tracks.lock().unwrap();
        finished_tracks.clear();
        drop(finished_tracks);

        self.abort = Arc::new(AtomicBool::new(false));
        self.playback_thread_exists.store(true, Ordering::SeqCst);
        let playback_id = self.playback_id.fetch_add(1, Ordering::SeqCst) + 1;
        self.buffering_done.store(false, Ordering::SeqCst);
        let now_ms_value = now_ms();
        self.last_chunk_ms.store(now_ms_value, Ordering::Relaxed);
        self.last_time_update_ms
            .store(now_ms_value, Ordering::Relaxed);

        self.audio_heard.store(false, Ordering::Relaxed);
        self.output_meter.lock().unwrap().reset();

        let (output_mixer, opened_now) = {
            let mut output_stream = self.output_stream.lock().unwrap();
            let opened_now = if output_stream.is_none() {
                *output_stream = open_output_stream_with_retry();
                true
            } else {
                false
            };
            let Some(stream) = output_stream.as_ref() else {
                self.playback_thread_exists.store(false, Ordering::SeqCst);
                return;
            };
            (stream.mixer().clone(), opened_now)
        };
        if trace_ms > 0 {
            let elapsed_ms = now_ms().saturating_sub(trace_ms);
            if opened_now {
                debug!("play trace: output stream opened +{}ms", elapsed_ms);
            } else {
                debug!("play trace: output stream reused +{}ms", elapsed_ms);
            }
        }

        let context = ThreadContext {
            play_state: self.state.clone(),
            abort: self.abort.clone(),
            playback_thread_exists: self.playback_thread_exists.clone(),
            playback_id_atomic: self.playback_id.clone(),
            time_passed: self.ts.clone(),
            duration: self.duration.clone(),
            prot: self.prot.clone(),
            buffer_settings: self.buffer_settings.clone(),
            effects: self.effects.clone(),
            inline_effects_update: self.inline_effects_update.clone(),
            inline_track_mix_updates: self.inline_track_mix_updates.clone(),
            dsp_metrics: self.dsp_metrics.clone(),
            effects_reset: self.effects_reset.clone(),
            output_meter: self.output_meter.clone(),
            audio_info: self.info.clone(),
            next_resume_fade_ms: self.next_resume_fade_ms.clone(),
            end_of_stream_action: self.end_of_stream_action.clone(),
            audio_heard: self.audio_heard.clone(),
            play_command_ms: self.play_command_ms.clone(),
            volume: self.volume.clone(),
            sink_mutex: self.sink.clone(),
            output_mixer,
            buffer_done_thread_flag: self.buffering_done.clone(),
            last_chunk_ms: self.last_chunk_ms.clone(),
            last_time_update_ms: self.last_time_update_ms.clone(),
        };

        let handle = thread::spawn(move || run_playback_thread(context, playback_id, ts));
        *self.playback_thread_handle.lock().unwrap() = Some(handle);
        if trace_ms > 0 {
            debug!(
                "play trace: initialize_thread spawned playback_id={} +{}ms",
                playback_id,
                now_ms().saturating_sub(trace_ms)
            );
        } else {
            debug!(
                "play trace: initialize_thread spawned playback_id={}",
                playback_id
            );
        }
    }
}
