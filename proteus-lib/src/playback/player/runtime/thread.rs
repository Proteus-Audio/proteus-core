//! Playback-thread bootstrap for `Player`.
//!
//! This module prepares shared state, resets per-run counters, and spawns the
//! worker loop that performs decoding handoff and sink append operations.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use super::super::Player;
use super::now_ms;
use super::worker::{run_playback_thread, ThreadContext};

impl Player {
    /// Initialize and spawn a fresh playback thread.
    ///
    /// # Arguments
    ///
    /// * `ts` - Optional starting time in seconds. When `None`, playback starts
    ///   from `0.0`.
    pub(in crate::playback::player) fn initialize_thread(&mut self, ts: Option<f64>) {
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

        let context = ThreadContext {
            play_state: self.state.clone(),
            abort: self.abort.clone(),
            playback_thread_exists: self.playback_thread_exists.clone(),
            playback_id_atomic: self.playback_id.clone(),
            time_passed: self.ts.clone(),
            duration: self.duration.clone(),
            prot: self.prot.clone(),
            buffer_settings: self.buffer_settings.clone(),
            buffer_settings_for_state: self.buffer_settings.clone(),
            effects: self.effects.clone(),
            inline_effects_update: self.inline_effects_update.clone(),
            inline_track_mix_updates: self.inline_track_mix_updates.clone(),
            dsp_metrics: self.dsp_metrics.clone(),
            dsp_metrics_for_sink: self.dsp_metrics.clone(),
            effects_reset: self.effects_reset.clone(),
            output_meter: self.output_meter.clone(),
            audio_info: self.info.clone(),
            next_resume_fade_ms: self.next_resume_fade_ms.clone(),
            audio_heard: self.audio_heard.clone(),
            volume: self.volume.clone(),
            sink_mutex: self.sink.clone(),
            buffer_done_thread_flag: self.buffering_done.clone(),
            last_chunk_ms: self.last_chunk_ms.clone(),
            last_time_update_ms: self.last_time_update_ms.clone(),
        };

        let handle = thread::spawn(move || run_playback_thread(context, playback_id, ts));
        *self.playback_thread_handle.lock().unwrap() = Some(handle);
    }
}
