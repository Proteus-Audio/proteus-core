//! Playback-state transition helpers for the worker loop.

use rodio::Sink;
use std::sync::atomic::Ordering;
use std::time::Instant;

use super::context::ThreadContext;
use super::runner::LoopState;
use super::sink::{pause_sink, resume_sink};
use super::timing::play_trace_elapsed_ms;
use crate::playback::player::runtime::now_ms;
use crate::playback::player::{EndOfStreamAction, PlayerState};
use log::debug;

// Poll control/abort state and apply transport transitions.
pub(super) fn check_runtime_state(ctx: &ThreadContext, loop_state: &mut LoopState) -> bool {
    if handle_abort(ctx, loop_state) {
        return false;
    }

    let state = *ctx.lock_play_state_invariant();
    let start_sink_chunks = ctx.lock_buffer_settings_recoverable().start_sink_chunks;
    let sink = ctx.lock_sink_recoverable();

    if handle_resuming_gate(state, start_sink_chunks, &sink, loop_state) {
        return true;
    }
    if handle_pausing(ctx, loop_state, state, &sink) {
        return true;
    }
    if handle_resuming_commit(ctx, loop_state, state, start_sink_chunks, &sink) {
        return true;
    }

    loop_state.resuming_gate_started_at = None;
    true
}

fn handle_abort(ctx: &ThreadContext, loop_state: &mut LoopState) -> bool {
    if !ctx.abort.load(Ordering::SeqCst) {
        return false;
    }
    let sink = ctx.lock_sink_recoverable();
    pause_sink(ctx, loop_state, &sink, 0.1);
    sink.clear();
    true
}

fn handle_resuming_gate(
    state: PlayerState,
    start_sink_chunks: usize,
    sink: &Sink,
    loop_state: &mut LoopState,
) -> bool {
    if state != PlayerState::Resuming || start_sink_chunks == 0 || sink.len() >= start_sink_chunks {
        return false;
    }
    if loop_state.resuming_gate_started_at.is_none() {
        loop_state.resuming_gate_started_at = Some(Instant::now());
    }
    sink.pause();
    true
}

fn handle_pausing(
    ctx: &ThreadContext,
    loop_state: &mut LoopState,
    state: PlayerState,
    sink: &Sink,
) -> bool {
    if state != PlayerState::Pausing {
        return false;
    }
    pause_sink(ctx, loop_state, sink, 0.1);
    ctx.lock_play_state_invariant()
        .clone_from(&PlayerState::Paused);
    true
}

fn handle_resuming_commit(
    ctx: &ThreadContext,
    loop_state: &mut LoopState,
    state: PlayerState,
    start_sink_chunks: usize,
    sink: &Sink,
) -> bool {
    if state != PlayerState::Resuming {
        return false;
    }
    let resume_gate_wait_ms = loop_state
        .resuming_gate_started_at
        .take()
        .map(|start| start.elapsed().as_millis())
        .unwrap_or(0);
    if let Some(elapsed_ms) = play_trace_elapsed_ms(ctx) {
        debug!(
            "play trace: resuming gate passed sink_len={} start_sink_chunks={} gate_wait_ms={} +{}ms",
            sink.len(),
            start_sink_chunks,
            resume_gate_wait_ms,
            elapsed_ms
        );
    }
    let fade_length = if loop_state.startup_fade_pending {
        loop_state.startup_fade_pending = false;
        if let Some(ms) = ctx.lock_next_resume_fade_ms_recoverable().take() {
            (ms / 1000.0).max(0.0)
        } else {
            (ctx.lock_buffer_settings_recoverable().startup_fade_ms / 1000.0).max(0.0)
        }
    } else {
        0.1
    };

    resume_sink(ctx, sink, fade_length);
    ctx.lock_play_state_invariant()
        .clone_from(&PlayerState::Playing);
    true
}

// Apply the configured transport action after a natural end-of-stream.
pub(super) fn apply_end_of_stream_action(ctx: &ThreadContext, loop_state: &LoopState) {
    let action = *ctx.lock_end_of_stream_action_recoverable();
    let duration = *ctx.lock_duration_recoverable();
    let final_duration = *loop_state.lock_final_duration_recoverable();

    match action {
        EndOfStreamAction::Stop => {
            ctx.lock_play_state_invariant()
                .clone_from(&PlayerState::Stopped);
            {
                let sink = ctx.lock_sink_recoverable();
                sink.stop();
                sink.clear();
            }
            *ctx.lock_time_passed_recoverable() = 0.0;
        }
        EndOfStreamAction::Pause => {
            ctx.lock_play_state_invariant()
                .clone_from(&PlayerState::Paused);
            {
                let sink = ctx.lock_sink_recoverable();
                sink.pause();
            }
            let target_end = match final_duration {
                Some(value) if value.is_finite() && value >= 0.0 => value,
                _ => duration.max(0.0),
            };
            *ctx.lock_time_passed_recoverable() = target_end;
        }
    }

    ctx.last_time_update_ms.store(now_ms(), Ordering::Relaxed);
}
