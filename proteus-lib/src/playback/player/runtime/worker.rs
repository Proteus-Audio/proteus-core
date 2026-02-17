//! Playback worker loop.
//!
//! This module owns the long-running runtime loop that receives mixed chunks
//! from `PlayerEngine`, appends them to `rodio::Sink`, and continuously updates
//! time/meter/debug bookkeeping.

use rodio::buffer::SamplesBuffer;
use rodio::{OutputStreamBuilder, Sink};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc::RecvTimeoutError, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use log::{error, warn};

use crate::container::info::Info;
use crate::container::prot::Prot;
use crate::dsp::effects::AudioEffect;
use crate::playback::engine::{
    DspChainMetrics, InlineEffectsUpdate, PlaybackBufferSettings, PlayerEngine,
};
use crate::playback::output_meter::OutputMeter;
use crate::tools::timer;

use super::super::{PlayerState, OUTPUT_STREAM_OPEN_RETRIES, OUTPUT_STREAM_OPEN_RETRY_MS};
use super::now_ms;

/// Captured shared state passed from `Player::initialize_thread` into the
/// detached worker thread.
pub(super) struct ThreadContext {
    pub(super) play_state: Arc<Mutex<PlayerState>>,
    pub(super) abort: Arc<AtomicBool>,
    pub(super) playback_thread_exists: Arc<AtomicBool>,
    pub(super) playback_id_atomic: Arc<AtomicU64>,
    pub(super) time_passed: Arc<Mutex<f64>>,
    pub(super) duration: Arc<Mutex<f64>>,
    pub(super) prot: Arc<Mutex<Prot>>,
    pub(super) buffer_settings: Arc<Mutex<PlaybackBufferSettings>>,
    pub(super) buffer_settings_for_state: Arc<Mutex<PlaybackBufferSettings>>,
    pub(super) effects: Arc<Mutex<Vec<AudioEffect>>>,
    pub(super) inline_effects_update: Arc<Mutex<Option<InlineEffectsUpdate>>>,
    pub(super) dsp_metrics: Arc<Mutex<DspChainMetrics>>,
    pub(super) dsp_metrics_for_sink: Arc<Mutex<DspChainMetrics>>,
    pub(super) effects_reset: Arc<AtomicU64>,
    pub(super) output_meter: Arc<Mutex<OutputMeter>>,
    pub(super) audio_info: Info,
    pub(super) next_resume_fade_ms: Arc<Mutex<Option<f32>>>,
    pub(super) audio_heard: Arc<AtomicBool>,
    pub(super) volume: Arc<Mutex<f32>>,
    pub(super) sink_mutex: Arc<Mutex<Sink>>,
    pub(super) buffer_done_thread_flag: Arc<AtomicBool>,
    pub(super) last_chunk_ms: Arc<AtomicU64>,
    pub(super) last_time_update_ms: Arc<AtomicU64>,
}

/// Guard that keeps `playback_thread_exists` in sync with worker lifetime.
struct PlaybackThreadGuard {
    exists: Arc<AtomicBool>,
}

impl PlaybackThreadGuard {
    fn new(exists: Arc<AtomicBool>) -> Self {
        exists.store(true, Ordering::Relaxed);
        Self { exists }
    }
}

impl Drop for PlaybackThreadGuard {
    fn drop(&mut self) {
        self.exists.store(false, Ordering::Relaxed);
    }
}

/// Run the playback worker loop for a single generation (`playback_id`).
///
/// # Arguments
///
/// * `ctx` - Captured shared state and handles for this run.
/// * `playback_id` - Generation ID used to invalidate stale workers.
/// * `ts` - Optional start timestamp in seconds.
pub(super) fn run_playback_thread(ctx: ThreadContext, playback_id: u64, ts: Option<f64>) {
    let thread_guard = PlaybackThreadGuard::new(ctx.playback_thread_exists.clone());

    let start_time = ts.unwrap_or(0.0);
    let mut engine = PlayerEngine::new(
        ctx.prot,
        Some(ctx.abort.clone()),
        start_time,
        ctx.buffer_settings,
        ctx.effects,
        ctx.dsp_metrics,
        ctx.effects_reset,
        ctx.inline_effects_update,
    );
    let mut stream = None;
    for attempt in 1..=OUTPUT_STREAM_OPEN_RETRIES {
        match OutputStreamBuilder::open_default_stream() {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(err) => {
                if attempt == OUTPUT_STREAM_OPEN_RETRIES {
                    error!(
                        "failed to open default output stream after {} attempts: {}",
                        OUTPUT_STREAM_OPEN_RETRIES, err
                    );
                    return;
                }
                warn!(
                    "open_default_stream attempt {}/{} failed: {}",
                    attempt, OUTPUT_STREAM_OPEN_RETRIES, err
                );
                thread::sleep(Duration::from_millis(OUTPUT_STREAM_OPEN_RETRY_MS));
            }
        }
    }
    let stream = stream.expect("stream should exist after successful retry loop");
    let mixer = stream.mixer().clone();

    let mut sink = ctx.sink_mutex.lock().unwrap();
    *sink = Sink::connect_new(&mixer);
    sink.pause();
    sink.set_volume(*ctx.volume.lock().unwrap());
    drop(sink);

    let mut duration = ctx.duration.lock().unwrap();
    *duration = engine.get_duration();
    drop(duration);

    let chunk_lengths = Arc::new(Mutex::new(Vec::new()));
    let mut time_passed_unlocked = ctx.time_passed.lock().unwrap();
    *time_passed_unlocked = start_time;
    drop(time_passed_unlocked);

    let pause_sink = |sink: &Sink, fade_length_out_seconds: f32| {
        let timestamp = *ctx.time_passed.lock().unwrap();

        let fade_increments = sink.volume() / (fade_length_out_seconds * 100.0);
        while sink.volume() > 0.0 && timestamp != start_time {
            sink.set_volume(sink.volume() - fade_increments);
            thread::sleep(Duration::from_millis(10));
        }
        sink.pause();
    };

    let resume_sink = |sink: &Sink, fade_length_in_seconds: f32| {
        let volume = *ctx.volume.lock().unwrap();
        if fade_length_in_seconds <= 0.0 {
            sink.play();
            sink.set_volume(volume);
            return;
        }
        let fade_increments = (volume - sink.volume()) / (fade_length_in_seconds * 100.0);
        sink.play();
        while sink.volume() < volume {
            sink.set_volume(sink.volume() + fade_increments);
            thread::sleep(Duration::from_millis(5));
        }
    };

    {
        let startup_settings = ctx.buffer_settings_for_state.lock().unwrap();
        let startup_silence_ms = startup_settings.startup_silence_ms;
        drop(startup_settings);

        let sample_rate = ctx.audio_info.sample_rate as u32;
        let channels = ctx.audio_info.channels as u16;

        if startup_silence_ms > 0.0 {
            let samples = ((startup_silence_ms / 1000.0) * sample_rate as f32).ceil() as usize
                * channels as usize;
            let silence = vec![0.0_f32; samples.max(1)];
            let silence_buffer = SamplesBuffer::new(channels, sample_rate, silence);
            let sink = ctx.sink_mutex.lock().unwrap();
            sink.append(silence_buffer);
            drop(sink);
        }
    }

    let startup_fade_pending = Cell::new(true);
    let check_details = || {
        if ctx.abort.load(Ordering::SeqCst) {
            let sink = ctx.sink_mutex.lock().unwrap();
            pause_sink(&sink, 0.1);
            sink.clear();
            drop(sink);

            return false;
        }

        let sink = ctx.sink_mutex.lock().unwrap();
        let state = ctx.play_state.lock().unwrap().clone();
        let start_sink_chunks = ctx
            .buffer_settings_for_state
            .lock()
            .unwrap()
            .start_sink_chunks;
        if state == PlayerState::Resuming && start_sink_chunks > 0 && sink.len() < start_sink_chunks
        {
            sink.pause();
            drop(sink);
            return true;
        }
        if state == PlayerState::Pausing {
            pause_sink(&sink, 0.1);
            ctx.play_state
                .lock()
                .unwrap()
                .clone_from(&PlayerState::Paused);
        }
        if state == PlayerState::Resuming {
            let fade_length = if startup_fade_pending.replace(false) {
                if let Some(ms) = ctx.next_resume_fade_ms.lock().unwrap().take() {
                    (ms / 1000.0).max(0.0)
                } else {
                    let startup_fade_ms = ctx
                        .buffer_settings_for_state
                        .lock()
                        .unwrap()
                        .startup_fade_ms;
                    (startup_fade_ms / 1000.0).max(0.0)
                }
            } else {
                0.1
            };
            resume_sink(&sink, fade_length);
            ctx.play_state
                .lock()
                .unwrap()
                .clone_from(&PlayerState::Playing);
        }
        drop(sink);

        true
    };

    let time_chunks_mutex = Arc::new(Mutex::new(start_time));
    let timer_mut = Arc::new(Mutex::new(timer::Timer::new()));
    let buffering_done = Arc::new(AtomicBool::new(false));
    let buffering_done_flag = ctx.buffer_done_thread_flag.clone();
    let final_duration = Arc::new(Mutex::new(None::<f64>));
    let mut timer = timer_mut.lock().unwrap();
    timer.start();
    drop(timer);

    let last_meter_time = Cell::new(0.0_f64);
    let update_chunk_lengths = || {
        if ctx.abort.load(Ordering::SeqCst) {
            return;
        }

        let mut chunk_lengths = chunk_lengths.lock().unwrap();
        let mut time_passed_unlocked = ctx.time_passed.lock().unwrap();
        let mut time_chunks_passed = time_chunks_mutex.lock().unwrap();
        let mut timer = timer_mut.lock().unwrap();
        ctx.last_time_update_ms.store(now_ms(), Ordering::Relaxed);
        let sink = ctx.sink_mutex.lock().unwrap();
        if !buffering_done.load(Ordering::Relaxed) {
            let chunks_played = chunk_lengths.len().saturating_sub(sink.len());

            for _ in 0..chunks_played {
                timer.reset();
                timer.start();
                *time_chunks_passed += chunk_lengths.remove(0);
            }
        }

        if sink.is_paused() {
            timer.pause();
        } else {
            timer.un_pause();
        }

        let current_audio_time = *time_chunks_passed + timer.get_time().as_secs_f64();
        let delta = (current_audio_time - last_meter_time.get()).max(0.0);
        last_meter_time.set(current_audio_time);
        {
            let mut meter = ctx.output_meter.lock().unwrap();
            meter.advance(delta);
        }

        *time_passed_unlocked = current_audio_time;

        drop(sink);
        drop(chunk_lengths);
        drop(time_passed_unlocked);
        drop(time_chunks_passed);
        drop(timer);
    };

    let append_timing = Arc::new(Mutex::new((Instant::now(), 0.0_f64, 0_u64, 0.0_f64)));
    let update_sink = |(mixer, length_in_seconds): (SamplesBuffer, f64)| {
        if ctx.playback_id_atomic.load(Ordering::SeqCst) != playback_id {
            return;
        }
        let max_sink_chunks = {
            let settings = ctx.buffer_settings_for_state.lock().unwrap();
            settings.max_sink_chunks
        };
        if max_sink_chunks > 0 {
            loop {
                if ctx.abort.load(Ordering::SeqCst) {
                    return;
                }
                if ctx.playback_id_atomic.load(Ordering::SeqCst) != playback_id {
                    return;
                }
                let sink_len = {
                    let sink = ctx.sink_mutex.lock().unwrap();
                    sink.len()
                };
                if sink_len < max_sink_chunks {
                    break;
                }
                update_chunk_lengths();
                if !check_details() {
                    return;
                }
                thread::sleep(Duration::from_millis(5));
            }
        }
        let (delay_ms, late) = {
            let mut timing = append_timing.lock().unwrap();
            let now = Instant::now();
            let delta_ms = now.duration_since(timing.0).as_secs_f64() * 1000.0;
            let chunk_ms = length_in_seconds * 1000.0;
            let late = delta_ms > (chunk_ms * 1.2) && chunk_ms > 0.0;
            if late {
                timing.2 = timing.2.saturating_add(1);
            }
            timing.1 = if timing.1 == 0.0 {
                delta_ms
            } else {
                (timing.1 * 0.9) + (delta_ms * 0.1)
            };
            timing.3 = timing.3.max(delta_ms);
            timing.0 = now;
            (delta_ms, late)
        };
        ctx.audio_heard.store(true, Ordering::Relaxed);
        ctx.last_chunk_ms.store(now_ms(), Ordering::Relaxed);

        {
            let mut meter = ctx.output_meter.lock().unwrap();
            meter.push_samples(&mixer);
        }
        {
            let mut metrics = ctx.dsp_metrics_for_sink.lock().unwrap();
            metrics.append_delay_ms = delay_ms;
            metrics.avg_append_delay_ms = {
                if metrics.avg_append_delay_ms == 0.0 {
                    delay_ms
                } else {
                    (metrics.avg_append_delay_ms * 0.9) + (delay_ms * 0.1)
                }
            };
            metrics.max_append_delay_ms = metrics.max_append_delay_ms.max(delay_ms);
            metrics.late_append_count = {
                let timing = append_timing.lock().unwrap();
                timing.2
            };
            metrics.late_append_active = late;
        }

        let sink = ctx.sink_mutex.lock().unwrap();
        let append_jitter_log_ms = {
            let settings = ctx.buffer_settings_for_state.lock().unwrap();
            settings.append_jitter_log_ms
        };
        if append_jitter_log_ms > 0.0 && (late || delay_ms > append_jitter_log_ms as f64) {
            let expected_ms = length_in_seconds * 1000.0;
            log::info!(
                "append jitter: delta={:.2}ms expected={:.2}ms late={} threshold={:.2}ms sink_len={}",
                delay_ms,
                expected_ms,
                late,
                append_jitter_log_ms,
                sink.len()
            );
        }
        let mut chunk_lengths = chunk_lengths.lock().unwrap();

        sink.append(mixer);

        drop(sink);

        chunk_lengths.push(length_in_seconds);
        drop(chunk_lengths);

        update_chunk_lengths();
        check_details();
    };

    let receiver = engine.start_receiver();
    loop {
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(chunk) => {
                update_sink(chunk);
            }
            Err(RecvTimeoutError::Timeout) => {
                update_chunk_lengths();
                if !check_details() {
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    #[cfg(feature = "debug")]
    log::info!("engine reception loop finished");

    buffering_done.store(true, Ordering::Relaxed);
    buffering_done_flag.store(true, Ordering::Relaxed);
    {
        let mut final_duration = final_duration.lock().unwrap();
        if final_duration.is_none() {
            let chunk_lengths = chunk_lengths.lock().unwrap();
            let time_chunks_passed = time_chunks_mutex.lock().unwrap();
            *final_duration = Some(*time_chunks_passed + chunk_lengths.iter().sum::<f64>());
        }
    }

    #[cfg(feature = "debug")]
    {
        let sink = ctx.sink_mutex.lock().unwrap();
        let paused = sink.is_paused();
        let empty = sink.empty();
        let sink_len = sink.len();
        drop(sink);
        let time_passed = *ctx.time_passed.lock().unwrap();
        let final_duration = *final_duration.lock().unwrap();
        log::info!(
            "Starting drain loop: paused={} empty={} sink_len={} time={:.3} final={:?}",
            paused,
            empty,
            sink_len,
            time_passed,
            final_duration
        );
    }

    loop {
        update_chunk_lengths();
        if !check_details() {
            break;
        }

        let done = if engine.finished_buffering() {
            if let Some(final_duration) = *final_duration.lock().unwrap() {
                let time_passed = *ctx.time_passed.lock().unwrap();
                time_passed >= (final_duration - 0.001).max(0.0)
            } else {
                false
            }
        } else {
            false
        };
        if done {
            break;
        }

        thread::sleep(Duration::from_millis(10));
    }

    #[cfg(feature = "debug")]
    log::info!("Finished drain loop!");

    drop(thread_guard);
}
