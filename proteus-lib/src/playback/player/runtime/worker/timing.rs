//! Timing, drain, and diagnostics helpers for the worker loop.

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use crate::playback::engine::PlayerEngine;
use crate::playback::player::runtime::now_ms;
use crate::tools::timer;

use super::context::ThreadContext;
use super::runner::LoopState;
use super::transitions::check_runtime_state;

// Advance playback clock/meter state from sink and timer progress.
pub(super) fn update_chunk_lengths(ctx: &ThreadContext, loop_state: &mut LoopState) {
    if ctx.abort.load(Ordering::SeqCst) {
        return;
    }

    let mut chunk_lengths = loop_state.chunk_lengths.lock().unwrap_or_else(|_| {
        panic!("chunk lengths lock poisoned — a thread panicked while holding it")
    });
    let mut time_passed_unlocked = ctx.time_passed.lock().unwrap_or_else(|_| {
        panic!("time passed lock poisoned — a thread panicked while holding it")
    });
    let mut time_chunks_passed = loop_state.time_chunks_passed.lock().unwrap_or_else(|_| {
        panic!("time chunks passed lock poisoned — a thread panicked while holding it")
    });
    let mut timer = loop_state
        .timer
        .lock()
        .unwrap_or_else(|_| panic!("timer lock poisoned — a thread panicked while holding it"));
    let sink = ctx
        .sink_mutex
        .lock()
        .unwrap_or_else(|_| panic!("sink lock poisoned — a thread panicked while holding it"));

    ctx.last_time_update_ms.store(now_ms(), Ordering::Relaxed);

    let chunks_played = chunk_lengths.len().saturating_sub(sink.len());
    advance_playback_clock(
        chunks_played,
        &mut chunk_lengths,
        &mut time_chunks_passed,
        &mut timer,
    );

    if sink.is_paused() {
        timer.pause();
    } else {
        timer.un_pause();
    }

    let current_audio_time = *time_chunks_passed + timer.get_time().as_secs_f64();
    let delta = (current_audio_time - loop_state.last_meter_time).max(0.0);
    loop_state.last_meter_time = current_audio_time;
    ctx.output_meter
        .lock()
        .unwrap_or_else(|_| {
            panic!("output meter lock poisoned — a thread panicked while holding it")
        })
        .advance(delta);
    *time_passed_unlocked = current_audio_time;
}

fn advance_playback_clock(
    chunks_played: usize,
    chunk_lengths: &mut VecDeque<f64>,
    time_chunks_passed: &mut f64,
    timer: &mut timer::Timer,
) {
    for _ in 0..chunks_played {
        timer.reset();
        timer.start();
        if let Some(length) = chunk_lengths.pop_front() {
            *time_chunks_passed += length;
        }
    }
}

// Update append jitter statistics for one chunk.
pub(super) fn update_append_timing(loop_state: &LoopState, length_in_seconds: f64) -> (f64, bool) {
    let mut timing = loop_state.append_timing.lock().unwrap_or_else(|_| {
        panic!("append timing lock poisoned — a thread panicked while holding it")
    });
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

pub(super) fn play_trace_elapsed_ms(ctx: &ThreadContext) -> Option<u64> {
    let trace_ms = ctx.play_command_ms.load(Ordering::Relaxed);
    if trace_ms == 0 {
        None
    } else {
        Some(now_ms().saturating_sub(trace_ms))
    }
}

// Mark producer buffering complete and finalize expected drain duration.
//
// `ctx.buffer_done_thread_flag` is the cross-thread publication flag
// (`Player::buffering_done`). It is stored with `Release` so that external
// callers who load it with `Acquire` observe a consistent view of all worker
// state written before this point.
//
// `loop_state.buffering_done` is worker-thread-local; `Relaxed` is correct.
pub(super) fn mark_buffering_complete(ctx: &ThreadContext, loop_state: &LoopState) {
    loop_state.buffering_done.store(true, Ordering::Relaxed);
    // Release: publish buffering-complete event to external Acquire loads.
    ctx.buffer_done_thread_flag.store(true, Ordering::Release);

    let mut final_duration = loop_state.final_duration.lock().unwrap_or_else(|_| {
        panic!("final duration lock poisoned — a thread panicked while holding it")
    });
    if final_duration.is_none() {
        let chunk_lengths = loop_state.chunk_lengths.lock().unwrap_or_else(|_| {
            panic!("chunk lengths lock poisoned — a thread panicked while holding it")
        });
        let time_chunks_passed = loop_state.time_chunks_passed.lock().unwrap_or_else(|_| {
            panic!("time chunks passed lock poisoned — a thread panicked while holding it")
        });
        *final_duration = Some(*time_chunks_passed + chunk_lengths.iter().sum::<f64>());
    }
}

// Evaluate whether all buffered audio has drained from the sink.
pub(super) fn is_drain_complete(
    ctx: &ThreadContext,
    loop_state: &LoopState,
    engine: &PlayerEngine,
) -> bool {
    if !engine.finished_buffering() {
        return false;
    }

    if ctx
        .sink_mutex
        .lock()
        .unwrap_or_else(|_| panic!("sink lock poisoned — a thread panicked while holding it"))
        .empty()
    {
        return true;
    }

    if let Some(final_duration) = *loop_state.final_duration.lock().unwrap_or_else(|_| {
        panic!("final duration lock poisoned — a thread panicked while holding it")
    }) {
        let time_passed = *ctx.time_passed.lock().unwrap_or_else(|_| {
            panic!("time passed lock poisoned — a thread panicked while holding it")
        });
        return time_passed >= (final_duration + 0.25).max(0.0);
    }

    false
}

#[cfg(feature = "debug")]
pub(super) fn log_drain_loop_start(ctx: &ThreadContext, loop_state: &LoopState) {
    let sink = ctx
        .sink_mutex
        .lock()
        .unwrap_or_else(|_| panic!("sink lock poisoned — a thread panicked while holding it"));
    let paused = sink.is_paused();
    let empty = sink.empty();
    let sink_len = sink.len();
    drop(sink);

    let time_passed = *ctx.time_passed.lock().unwrap_or_else(|_| {
        panic!("time passed lock poisoned — a thread panicked while holding it")
    });
    let final_duration = *loop_state.final_duration.lock().unwrap_or_else(|_| {
        panic!("final duration lock poisoned — a thread panicked while holding it")
    });
    log::info!(
        "Starting drain loop: paused={} empty={} sink_len={} time={:.3} final={:?}",
        paused,
        empty,
        sink_len,
        time_passed,
        final_duration
    );
}

pub(super) fn run_drain_loop(
    ctx: &ThreadContext,
    loop_state: &mut LoopState,
    engine: &PlayerEngine,
) -> bool {
    loop {
        update_chunk_lengths(ctx, loop_state);
        if !check_runtime_state(ctx, loop_state) {
            return false;
        }

        if is_drain_complete(ctx, loop_state, engine) {
            return true;
        }

        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::update_append_timing;
    use crate::playback::player::runtime::worker::runner::LoopState;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn update_append_timing_marks_late_when_delta_exceeds_threshold() {
        let state = LoopState::new(0.0);
        {
            let mut timing = state.append_timing.lock().unwrap();
            timing.0 = std::time::Instant::now() - Duration::from_millis(100);
        }

        let (delay_ms, late) = update_append_timing(&state, 0.01);
        assert!(delay_ms >= 80.0);
        assert!(late);
        assert_eq!(state.append_timing.lock().unwrap().2, 1);
    }

    #[test]
    fn update_append_timing_tracks_average_without_late_increment() {
        let state = LoopState::new(0.0);
        let _ = update_append_timing(&state, 0.5);
        {
            let mut timing = state.append_timing.lock().unwrap();
            timing.0 = std::time::Instant::now() - Duration::from_millis(12);
        }
        let (_, late) = update_append_timing(&state, 0.5);
        assert!(!late);
        let timing = state.append_timing.lock().unwrap();
        assert!(timing.1 > 0.0);
        assert_eq!(timing.2, 0);
        assert!(timing.3 >= timing.1);
    }

    // Verify that a Release store to buffer_done_thread_flag (the cross-thread
    // publication path used by mark_buffering_complete) is visible to an Acquire
    // load on the observing thread.
    #[test]
    fn buffering_done_publication_visible_across_threads() {
        let flag = Arc::new(AtomicBool::new(false));
        let writer = flag.clone();
        let handle = std::thread::spawn(move || {
            // Mirrors the store in mark_buffering_complete.
            writer.store(true, Ordering::Release);
        });
        handle.join().unwrap();
        // Mirrors the load in debug_buffering_done.
        assert!(flag.load(Ordering::Acquire));
    }

    // Verify that the spawner-side reset (false with Release) is visible before
    // the worker thread begins execution.
    #[test]
    fn buffering_done_reset_before_spawn_is_false() {
        let flag = Arc::new(AtomicBool::new(true));
        // Spawner resets to false before spawning.
        flag.store(false, Ordering::Release);
        let reader = flag.clone();
        let handle = std::thread::spawn(move || {
            // Worker reads the spawner's reset; Acquire ensures it sees false.
            reader.load(Ordering::Acquire)
        });
        let seen = handle.join().unwrap();
        assert!(!seen);
    }
}
