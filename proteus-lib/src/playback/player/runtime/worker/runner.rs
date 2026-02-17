//! Playback worker loop implementation.

use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, OutputStreamBuilder, Sink};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc::RecvTimeoutError, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use log::{error, warn};

use crate::playback::engine::PlayerEngine;
use crate::tools::timer;

use super::super::super::{PlayerState, OUTPUT_STREAM_OPEN_RETRIES, OUTPUT_STREAM_OPEN_RETRY_MS};
use super::super::now_ms;
use super::context::ThreadContext;
use super::guard::PlaybackThreadGuard;

/// Per-run mutable state for playback time, buffering, and append timing.
struct LoopState {
    start_time: f64,
    startup_fade_pending: bool,
    chunk_lengths: Arc<Mutex<Vec<f64>>>,
    time_chunks_passed: Arc<Mutex<f64>>,
    timer: Arc<Mutex<timer::Timer>>,
    buffering_done: Arc<AtomicBool>,
    final_duration: Arc<Mutex<Option<f64>>>,
    last_meter_time: f64,
    append_timing: Arc<Mutex<(Instant, f64, u64, f64)>>,
}

impl LoopState {
    /// Build initialized loop state for a new playback-thread run.
    ///
    /// # Arguments
    ///
    /// * `start_time` - Initial playback position in seconds.
    fn new(start_time: f64) -> Self {
        let timer = Arc::new(Mutex::new(timer::Timer::new()));
        {
            let mut timer_guard = timer.lock().unwrap();
            timer_guard.start();
        }
        Self {
            start_time,
            startup_fade_pending: true,
            chunk_lengths: Arc::new(Mutex::new(Vec::new())),
            time_chunks_passed: Arc::new(Mutex::new(start_time)),
            timer,
            buffering_done: Arc::new(AtomicBool::new(false)),
            final_duration: Arc::new(Mutex::new(None)),
            last_meter_time: 0.0,
            append_timing: Arc::new(Mutex::new((Instant::now(), 0.0, 0, 0.0))),
        }
    }
}

/// Run the playback worker loop for a single generation (`playback_id`).
///
/// # Arguments
///
/// * `ctx` - Captured shared state and handles for this run.
/// * `playback_id` - Generation ID used to invalidate stale workers.
/// * `ts` - Optional start timestamp in seconds.
pub(in crate::playback::player::runtime) fn run_playback_thread(
    ctx: ThreadContext,
    playback_id: u64,
    ts: Option<f64>,
) {
    let _thread_guard = PlaybackThreadGuard::new(ctx.playback_thread_exists.clone());
    let start_time = ts.unwrap_or(0.0);

    let mut engine = PlayerEngine::new(
        ctx.prot.clone(),
        Some(ctx.abort.clone()),
        start_time,
        ctx.buffer_settings.clone(),
        ctx.effects.clone(),
        ctx.dsp_metrics.clone(),
        ctx.effects_reset.clone(),
        ctx.inline_effects_update.clone(),
    );

    let stream = match open_output_stream_with_retry() {
        Some(stream) => stream,
        None => return,
    };
    let mixer = stream.mixer().clone();

    initialize_sink(&ctx, &mixer);
    set_duration_from_engine(&ctx, &engine);
    set_start_time(&ctx, start_time);
    append_startup_silence(&ctx);

    let mut loop_state = LoopState::new(start_time);

    let receiver = engine.start_receiver();
    loop {
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(chunk) => {
                update_sink(&ctx, &mut loop_state, playback_id, chunk);
            }
            Err(RecvTimeoutError::Timeout) => {
                update_chunk_lengths(&ctx, &mut loop_state);
                if !check_runtime_state(&ctx, &mut loop_state) {
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    #[cfg(feature = "debug")]
    log::info!("engine reception loop finished");

    mark_buffering_complete(&ctx, &loop_state);

    #[cfg(feature = "debug")]
    log_drain_loop_start(&ctx, &loop_state);

    loop {
        update_chunk_lengths(&ctx, &mut loop_state);
        if !check_runtime_state(&ctx, &mut loop_state) {
            break;
        }

        if is_drain_complete(&ctx, &loop_state, &engine) {
            break;
        }

        thread::sleep(Duration::from_millis(10));
    }

    #[cfg(feature = "debug")]
    log::info!("Finished drain loop!");
}

/// Open the default output stream with bounded retry behavior.
///
/// # Returns
///
/// `Some(OutputStream)` on success, otherwise `None` after all retries fail.
fn open_output_stream_with_retry() -> Option<OutputStream> {
    for attempt in 1..=OUTPUT_STREAM_OPEN_RETRIES {
        match OutputStreamBuilder::open_default_stream() {
            Ok(stream) => return Some(stream),
            Err(err) => {
                if attempt == OUTPUT_STREAM_OPEN_RETRIES {
                    error!(
                        "failed to open default output stream after {} attempts: {}",
                        OUTPUT_STREAM_OPEN_RETRIES, err
                    );
                    return None;
                }
                warn!(
                    "open_default_stream attempt {}/{} failed: {}",
                    attempt, OUTPUT_STREAM_OPEN_RETRIES, err
                );
                thread::sleep(Duration::from_millis(OUTPUT_STREAM_OPEN_RETRY_MS));
            }
        }
    }
    None
}

/// Recreate and initialize the sink connected to the active mixer.
///
/// # Arguments
///
/// * `ctx` - Shared worker context with sink and volume state.
/// * `mixer` - Output mixer from the opened output stream.
fn initialize_sink(ctx: &ThreadContext, mixer: &rodio::mixer::Mixer) {
    let mut sink = ctx.sink_mutex.lock().unwrap();
    *sink = Sink::connect_new(mixer);
    sink.pause();
    sink.set_volume(*ctx.volume.lock().unwrap());
}

/// Snapshot the total engine duration into shared player state.
///
/// # Arguments
///
/// * `ctx` - Shared worker context containing duration state.
/// * `engine` - Active engine instance for this playback run.
fn set_duration_from_engine(ctx: &ThreadContext, engine: &PlayerEngine) {
    let mut duration = ctx.duration.lock().unwrap();
    *duration = engine.get_duration();
}

/// Initialize shared playback time to the selected start position.
///
/// # Arguments
///
/// * `ctx` - Shared worker context containing playback time state.
/// * `start_time` - Start position in seconds.
fn set_start_time(ctx: &ThreadContext, start_time: f64) {
    let mut time_passed = ctx.time_passed.lock().unwrap();
    *time_passed = start_time;
}

/// Append startup silence pre-roll when configured.
///
/// # Arguments
///
/// * `ctx` - Shared worker context with startup settings and sink handle.
fn append_startup_silence(ctx: &ThreadContext) {
    let startup_silence_ms = {
        let startup_settings = ctx.buffer_settings_for_state.lock().unwrap();
        startup_settings.startup_silence_ms
    };

    if startup_silence_ms <= 0.0 {
        return;
    }

    let sample_rate = ctx.audio_info.sample_rate as u32;
    let channels = ctx.audio_info.channels as u16;
    let samples =
        ((startup_silence_ms / 1000.0) * sample_rate as f32).ceil() as usize * channels as usize;
    let silence = vec![0.0_f32; samples.max(1)];
    let silence_buffer = SamplesBuffer::new(channels, sample_rate, silence);

    // Pre-roll silence gives output devices a stable startup window.
    let sink = ctx.sink_mutex.lock().unwrap();
    sink.append(silence_buffer);
}

/// Fade out and pause the sink.
///
/// # Arguments
///
/// * `ctx` - Shared worker context containing playback clock state.
/// * `loop_state` - Per-run loop state providing start-time reference.
/// * `sink` - Output sink to fade and pause.
/// * `fade_seconds` - Fade duration in seconds.
fn pause_sink(ctx: &ThreadContext, loop_state: &LoopState, sink: &Sink, fade_seconds: f32) {
    let timestamp = *ctx.time_passed.lock().unwrap();
    let fade_increments = sink.volume() / (fade_seconds * 100.0);

    while sink.volume() > 0.0 && timestamp != loop_state.start_time {
        sink.set_volume(sink.volume() - fade_increments);
        thread::sleep(Duration::from_millis(10));
    }
    sink.pause();
}

/// Start or resume sink playback with optional fade-in.
///
/// # Arguments
///
/// * `ctx` - Shared worker context containing target volume state.
/// * `sink` - Output sink to resume.
/// * `fade_seconds` - Fade duration in seconds.
fn resume_sink(ctx: &ThreadContext, sink: &Sink, fade_seconds: f32) {
    let target_volume = *ctx.volume.lock().unwrap();
    if fade_seconds <= 0.0 {
        sink.play();
        sink.set_volume(target_volume);
        return;
    }

    let fade_increments = (target_volume - sink.volume()) / (fade_seconds * 100.0);
    sink.play();
    while sink.volume() < target_volume {
        sink.set_volume(sink.volume() + fade_increments);
        thread::sleep(Duration::from_millis(5));
    }
}

/// Poll control/abort state and apply transport transitions.
///
/// # Arguments
///
/// * `ctx` - Shared worker context containing transport flags/states.
/// * `loop_state` - Mutable per-run state used by fade logic.
///
/// # Returns
///
/// `false` when the worker should terminate, otherwise `true`.
fn check_runtime_state(ctx: &ThreadContext, loop_state: &mut LoopState) -> bool {
    if ctx.abort.load(Ordering::SeqCst) {
        let sink = ctx.sink_mutex.lock().unwrap();
        pause_sink(ctx, loop_state, &sink, 0.1);
        sink.clear();
        return false;
    }

    let sink = ctx.sink_mutex.lock().unwrap();
    let state = *ctx.play_state.lock().unwrap();
    let start_sink_chunks = ctx
        .buffer_settings_for_state
        .lock()
        .unwrap()
        .start_sink_chunks;

    // Gate startup/resume until enough chunks are queued to reduce underflow.
    if state == PlayerState::Resuming && start_sink_chunks > 0 && sink.len() < start_sink_chunks {
        sink.pause();
        return true;
    }

    if state == PlayerState::Pausing {
        pause_sink(ctx, loop_state, &sink, 0.1);
        ctx.play_state
            .lock()
            .unwrap()
            .clone_from(&PlayerState::Paused);
    }

    if state == PlayerState::Resuming {
        let fade_length = if loop_state.startup_fade_pending {
            loop_state.startup_fade_pending = false;
            if let Some(ms) = ctx.next_resume_fade_ms.lock().unwrap().take() {
                (ms / 1000.0).max(0.0)
            } else {
                (ctx.buffer_settings_for_state
                    .lock()
                    .unwrap()
                    .startup_fade_ms
                    / 1000.0)
                    .max(0.0)
            }
        } else {
            0.1
        };

        resume_sink(ctx, &sink, fade_length);
        ctx.play_state
            .lock()
            .unwrap()
            .clone_from(&PlayerState::Playing);
    }

    true
}

/// Advance playback clock/meter state from sink and timer progress.
///
/// # Arguments
///
/// * `ctx` - Shared worker context with sink, meter, and timestamps.
/// * `loop_state` - Mutable per-run timing and chunk bookkeeping.
fn update_chunk_lengths(ctx: &ThreadContext, loop_state: &mut LoopState) {
    if ctx.abort.load(Ordering::SeqCst) {
        return;
    }

    let mut chunk_lengths = loop_state.chunk_lengths.lock().unwrap();
    let mut time_passed_unlocked = ctx.time_passed.lock().unwrap();
    let mut time_chunks_passed = loop_state.time_chunks_passed.lock().unwrap();
    let mut timer = loop_state.timer.lock().unwrap();
    let sink = ctx.sink_mutex.lock().unwrap();

    ctx.last_time_update_ms.store(now_ms(), Ordering::Relaxed);

    // While buffering is active, infer consumed chunks from sink queue depth.
    if !loop_state.buffering_done.load(Ordering::Relaxed) {
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
    let delta = (current_audio_time - loop_state.last_meter_time).max(0.0);
    loop_state.last_meter_time = current_audio_time;

    {
        let mut meter = ctx.output_meter.lock().unwrap();
        meter.advance(delta);
    }

    *time_passed_unlocked = current_audio_time;
}

/// Block append path until sink queue depth is below configured maximum.
///
/// # Arguments
///
/// * `ctx` - Shared worker context containing sink and buffering settings.
/// * `loop_state` - Mutable loop state used to keep time/state responsive.
/// * `playback_id` - Generation ID used to reject stale worker updates.
///
/// # Returns
///
/// `true` if appending may proceed, `false` if aborted/stale.
fn wait_for_sink_capacity(
    ctx: &ThreadContext,
    loop_state: &mut LoopState,
    playback_id: u64,
) -> bool {
    let max_sink_chunks = ctx
        .buffer_settings_for_state
        .lock()
        .unwrap()
        .max_sink_chunks;
    if max_sink_chunks == 0 {
        return true;
    }

    loop {
        if ctx.abort.load(Ordering::SeqCst) {
            return false;
        }
        if ctx.playback_id_atomic.load(Ordering::SeqCst) != playback_id {
            return false;
        }

        let sink_len = { ctx.sink_mutex.lock().unwrap().len() };
        if sink_len < max_sink_chunks {
            return true;
        }

        update_chunk_lengths(ctx, loop_state);
        if !check_runtime_state(ctx, loop_state) {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

/// Update append jitter statistics for one chunk.
///
/// # Arguments
///
/// * `loop_state` - Loop state storing rolling append timing stats.
/// * `length_in_seconds` - Duration of the chunk about to be appended.
///
/// # Returns
///
/// `(delay_ms, late)` for the current append interval.
fn update_append_timing(loop_state: &LoopState, length_in_seconds: f64) -> (f64, bool) {
    let mut timing = loop_state.append_timing.lock().unwrap();
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
}

/// Append one chunk to the sink and update runtime telemetry/state.
///
/// # Arguments
///
/// * `ctx` - Shared worker context with sink, metrics, and control state.
/// * `loop_state` - Mutable per-run timing/backpressure state.
/// * `playback_id` - Generation ID used to guard stale worker output.
/// * `chunk` - `(SamplesBuffer, duration_seconds)` emitted by the engine.
fn update_sink(
    ctx: &ThreadContext,
    loop_state: &mut LoopState,
    playback_id: u64,
    chunk: (SamplesBuffer, f64),
) {
    let (mixer, length_in_seconds) = chunk;

    if ctx.playback_id_atomic.load(Ordering::SeqCst) != playback_id {
        return;
    }
    if !wait_for_sink_capacity(ctx, loop_state, playback_id) {
        return;
    }

    let (delay_ms, late) = update_append_timing(loop_state, length_in_seconds);
    ctx.audio_heard.store(true, Ordering::Relaxed);
    ctx.last_chunk_ms.store(now_ms(), Ordering::Relaxed);

    {
        let mut meter = ctx.output_meter.lock().unwrap();
        meter.push_samples(&mixer);
    }

    {
        let mut metrics = ctx.dsp_metrics_for_sink.lock().unwrap();
        metrics.late_append_count = loop_state.append_timing.lock().unwrap().2;
        metrics.late_append_active = late;
    }

    let sink = ctx.sink_mutex.lock().unwrap();
    let append_jitter_log_ms = ctx
        .buffer_settings_for_state
        .lock()
        .unwrap()
        .append_jitter_log_ms;

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

    sink.append(mixer);
    drop(sink);
    loop_state
        .chunk_lengths
        .lock()
        .unwrap()
        .push(length_in_seconds);

    // Keep UI time and state responsive on every append.
    update_chunk_lengths(ctx, loop_state);
    check_runtime_state(ctx, loop_state);
}

/// Mark producer buffering complete and finalize expected drain duration.
///
/// # Arguments
///
/// * `ctx` - Shared worker context containing completion flags.
/// * `loop_state` - Loop state with chunk/time accumulators.
fn mark_buffering_complete(ctx: &ThreadContext, loop_state: &LoopState) {
    loop_state.buffering_done.store(true, Ordering::Relaxed);
    ctx.buffer_done_thread_flag.store(true, Ordering::Relaxed);

    let mut final_duration = loop_state.final_duration.lock().unwrap();
    if final_duration.is_none() {
        let chunk_lengths = loop_state.chunk_lengths.lock().unwrap();
        let time_chunks_passed = loop_state.time_chunks_passed.lock().unwrap();
        *final_duration = Some(*time_chunks_passed + chunk_lengths.iter().sum::<f64>());
    }
}

/// Evaluate whether all buffered audio has drained from the sink.
///
/// # Arguments
///
/// * `ctx` - Shared worker context containing current playback time.
/// * `loop_state` - Loop state containing computed final duration.
/// * `engine` - Engine used to check producer completion status.
///
/// # Returns
///
/// `true` once playback time reaches the final expected duration.
fn is_drain_complete(ctx: &ThreadContext, loop_state: &LoopState, engine: &PlayerEngine) -> bool {
    if !engine.finished_buffering() {
        return false;
    }

    if let Some(final_duration) = *loop_state.final_duration.lock().unwrap() {
        let time_passed = *ctx.time_passed.lock().unwrap();
        return time_passed >= (final_duration - 0.001).max(0.0);
    }

    false
}

#[cfg(feature = "debug")]
/// Emit a debug snapshot when entering the drain loop.
fn log_drain_loop_start(ctx: &ThreadContext, loop_state: &LoopState) {
    let sink = ctx.sink_mutex.lock().unwrap();
    let paused = sink.is_paused();
    let empty = sink.empty();
    let sink_len = sink.len();
    drop(sink);

    let time_passed = *ctx.time_passed.lock().unwrap();
    let final_duration = *loop_state.final_duration.lock().unwrap();
    log::info!(
        "Starting drain loop: paused={} empty={} sink_len={} time={:.3} final={:?}",
        paused,
        empty,
        sink_len,
        time_passed,
        final_duration
    );
}
