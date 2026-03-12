//! Sink and output-stream management helpers for the playback worker.

use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, OutputStreamBuilder, Sink};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use log::{debug, error, warn};

use super::context::ThreadContext;
use super::runner::LoopState;
use super::timing::{update_append_timing, update_chunk_lengths};
use super::transitions::check_runtime_state;
use crate::playback::player::runtime::now_ms;
use crate::playback::player::{OUTPUT_STREAM_OPEN_RETRIES, OUTPUT_STREAM_OPEN_RETRY_MS};

// Open the default output stream with bounded retry behavior.
//
// # Returns
//
// `Some(OutputStream)` on success, otherwise `None` after all retries fail.
pub(in crate::playback::player::runtime) fn open_output_stream_with_retry() -> Option<OutputStream>
{
    open_output_stream_with_retry_hooks(
        OUTPUT_STREAM_OPEN_RETRIES,
        OUTPUT_STREAM_OPEN_RETRY_MS,
        OutputStreamBuilder::open_default_stream,
        thread::sleep,
    )
}

fn open_output_stream_with_retry_hooks<Open, Sleep>(
    retries: usize,
    retry_ms: u64,
    mut open_fn: Open,
    mut sleep_fn: Sleep,
) -> Option<OutputStream>
where
    Open: FnMut() -> Result<OutputStream, rodio::StreamError>,
    Sleep: FnMut(Duration),
{
    for attempt in 1..=retries {
        match open_fn() {
            Ok(stream) => return Some(stream),
            Err(err) => {
                if attempt == retries {
                    error!(
                        "failed to open default output stream after {} attempts: {}",
                        retries, err
                    );
                    return None;
                }
                warn!(
                    "open_default_stream attempt {}/{} failed: {}",
                    attempt, retries, err
                );
                mut_sleep(&mut sleep_fn, retry_ms);
            }
        }
    }
    None
}

fn mut_sleep<Sleep>(sleep_fn: &mut Sleep, retry_ms: u64)
where
    Sleep: FnMut(Duration),
{
    sleep_fn(Duration::from_millis(retry_ms));
}

// Recreate and initialize the sink connected to the active mixer.
pub(super) fn initialize_sink(ctx: &ThreadContext, mixer: &rodio::mixer::Mixer) {
    let mut sink = ctx
        .sink_mutex
        .lock()
        .unwrap_or_else(|_| panic!("sink lock poisoned — a thread panicked while holding it"));
    *sink = Sink::connect_new(mixer);
    sink.pause();
    sink.set_volume(0.0);
}

// Append startup silence pre-roll when configured.
pub(super) fn append_startup_silence(ctx: &ThreadContext) {
    let startup_silence_ms = ctx
        .buffer_settings
        .lock()
        .unwrap_or_else(|_| {
            panic!("buffer settings lock poisoned — a thread panicked while holding it")
        })
        .startup_silence_ms;
    if startup_silence_ms <= 0.0 {
        return;
    }

    let sample_rate = ctx.audio_info.sample_rate;
    let channels = ctx.audio_info.channels as u16;
    let samples =
        ((startup_silence_ms / 1000.0) * sample_rate as f32).ceil() as usize * channels as usize;
    let silence_buffer = SamplesBuffer::new(channels, sample_rate, vec![0.0_f32; samples.max(1)]);
    ctx.sink_mutex
        .lock()
        .unwrap_or_else(|_| panic!("sink lock poisoned — a thread panicked while holding it"))
        .append(silence_buffer);
}

// Fade out and pause the sink.
pub(super) fn pause_sink(
    ctx: &ThreadContext,
    loop_state: &LoopState,
    sink: &Sink,
    fade_seconds: f32,
) {
    let timestamp = *ctx.time_passed.lock().unwrap_or_else(|_| {
        panic!("time passed lock poisoned — a thread panicked while holding it")
    });
    let fade_increments = sink.volume() / (fade_seconds * 100.0);

    while sink.volume() > 0.0 && timestamp != loop_state.start_time {
        sink.set_volume(sink.volume() - fade_increments);
        thread::sleep(Duration::from_millis(10));
    }
    sink.pause();
}

// Start or resume sink playback with optional fade-in.
pub(super) fn resume_sink(ctx: &ThreadContext, sink: &Sink, fade_seconds: f32) {
    let target_volume = *ctx
        .volume
        .lock()
        .unwrap_or_else(|_| panic!("volume lock poisoned — a thread panicked while holding it"));
    if let Some(elapsed_ms) = super::timing::play_trace_elapsed_ms(ctx) {
        debug!(
            "play trace: resume_sink begin fade_s={:.3} target_volume={:.3} +{}ms",
            fade_seconds, target_volume, elapsed_ms
        );
    }
    if fade_seconds <= 0.0 {
        sink.play();
        sink.set_volume(target_volume);
        if let Some(elapsed_ms) = super::timing::play_trace_elapsed_ms(ctx) {
            debug!(
                "play trace: resume_sink sink.play() immediate +{}ms",
                elapsed_ms
            );
        }
        return;
    }

    let mut current = sink.volume().clamp(0.0, target_volume);
    if (target_volume - current).abs() <= f32::EPSILON && target_volume > 0.0 {
        current = 0.0;
    }
    sink.set_volume(current);
    let fade_increments = ((target_volume - current) / (fade_seconds * 100.0)).max(0.000_001);
    sink.play();
    if let Some(elapsed_ms) = super::timing::play_trace_elapsed_ms(ctx) {
        debug!("play trace: resume_sink sink.play() +{}ms", elapsed_ms);
    }
    while sink.volume() < target_volume {
        sink.set_volume((sink.volume() + fade_increments).min(target_volume));
        thread::sleep(Duration::from_millis(5));
    }
    if let Some(elapsed_ms) = super::timing::play_trace_elapsed_ms(ctx) {
        debug!("play trace: resume_sink fade complete +{}ms", elapsed_ms);
    }
}

// Block append path until sink queue depth is below configured maximum.
pub(super) fn wait_for_sink_capacity(
    ctx: &ThreadContext,
    loop_state: &mut LoopState,
    playback_id: u64,
) -> bool {
    let max_sink_chunks = ctx
        .buffer_settings
        .lock()
        .unwrap_or_else(|_| {
            panic!("buffer settings lock poisoned — a thread panicked while holding it")
        })
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

        if ctx
            .sink_mutex
            .lock()
            .unwrap_or_else(|_| panic!("sink lock poisoned — a thread panicked while holding it"))
            .len()
            < max_sink_chunks
        {
            return true;
        }

        update_chunk_lengths(ctx, loop_state);
        if !check_runtime_state(ctx, loop_state) {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

// Append one chunk to the sink and update runtime telemetry/state.
pub(super) fn update_sink(
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
    let trace_ms = ctx.play_command_ms.load(Ordering::Relaxed);
    let now = now_ms();
    let prev_last_chunk_ms = ctx.last_chunk_ms.load(Ordering::Relaxed);
    if trace_ms > 0 && prev_last_chunk_ms < trace_ms {
        debug!(
            "play trace: first sink append after command chunk_ms={:.2} delay_ms={:.2} late={} +{}ms",
            length_in_seconds * 1000.0,
            delay_ms,
            late,
            now.saturating_sub(trace_ms)
        );
    }
    ctx.audio_heard.store(true, Ordering::Relaxed);
    ctx.last_chunk_ms.store(now, Ordering::Relaxed);

    ctx.output_meter
        .lock()
        .unwrap_or_else(|_| {
            panic!("output meter lock poisoned — a thread panicked while holding it")
        })
        .push_samples(&mixer);

    {
        let mut metrics = ctx.dsp_metrics.lock().unwrap_or_else(|_| {
            panic!("dsp metrics lock poisoned — a thread panicked while holding it")
        });
        metrics.late_append_count = loop_state
            .append_timing
            .lock()
            .unwrap_or_else(|_| {
                panic!("append timing lock poisoned — a thread panicked while holding it")
            })
            .2;
        metrics.late_append_active = late;
    }

    let sink = ctx
        .sink_mutex
        .lock()
        .unwrap_or_else(|_| panic!("sink lock poisoned — a thread panicked while holding it"));
    let append_jitter_log_ms = ctx
        .buffer_settings
        .lock()
        .unwrap_or_else(|_| {
            panic!("buffer settings lock poisoned — a thread panicked while holding it")
        })
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
        .unwrap_or_else(|_| {
            panic!("chunk lengths lock poisoned — a thread panicked while holding it")
        })
        .push_back(length_in_seconds);

    update_chunk_lengths(ctx, loop_state);
    check_runtime_state(ctx, loop_state);
}

#[cfg(test)]
mod tests {
    use super::open_output_stream_with_retry_hooks;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn open_output_stream_retry_hooks_exhaust_retries_without_sleeping_real_time() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let sleep_calls = Arc::new(AtomicUsize::new(0));
        let attempts_ref = attempts.clone();
        let sleep_ref = sleep_calls.clone();

        let stream = open_output_stream_with_retry_hooks(
            3,
            10,
            move || {
                attempts_ref.fetch_add(1, Ordering::Relaxed);
                Err(rodio::StreamError::NoDevice)
            },
            move |_| {
                sleep_ref.fetch_add(1, Ordering::Relaxed);
            },
        );

        assert!(stream.is_none());
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
        assert_eq!(sleep_calls.load(Ordering::Relaxed), 2);
    }
}
