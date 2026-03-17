//! Playback worker loop implementation.

use rodio::buffer::SamplesBuffer;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc::RecvTimeoutError, Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use log::debug;

use crate::playback::engine::{PlayerEngine, PlayerEngineConfig};
use crate::playback::mutex_policy::lock_recoverable;
use crate::tools::timer;

use super::context::ThreadContext;
use super::guard::PlaybackThreadGuard;
use super::sink::{append_startup_silence, initialize_sink, update_sink};
#[cfg(feature = "debug")]
use super::timing::log_drain_loop_start;
use super::timing::{
    mark_buffering_complete, play_trace_elapsed_ms, run_drain_loop, update_chunk_lengths,
};
use super::transitions::{apply_end_of_stream_action, check_runtime_state};

// Per-run mutable state for playback time, buffering, and append timing.
pub(super) struct LoopState {
    pub(super) start_time: f64,
    pub(super) startup_fade_pending: bool,
    pub(super) chunk_lengths: Arc<Mutex<VecDeque<f64>>>,
    pub(super) time_chunks_passed: Arc<Mutex<f64>>,
    pub(super) timer: Arc<Mutex<timer::Timer>>,
    pub(super) buffering_done: Arc<AtomicBool>,
    pub(super) final_duration: Arc<Mutex<Option<f64>>>,
    pub(super) last_meter_time: f64,
    pub(super) append_timing: Arc<Mutex<(Instant, f64, u64, f64)>>,
    pub(super) resuming_gate_started_at: Option<Instant>,
}

impl LoopState {
    pub(super) fn new(start_time: f64) -> Self {
        let timer = Arc::new(Mutex::new(timer::Timer::new()));
        {
            let mut timer_guard = lock_recoverable(
                &timer,
                "playback timer",
                "the timer is disposable runtime coordination state",
            );
            timer_guard.start();
        }
        Self {
            start_time,
            startup_fade_pending: true,
            chunk_lengths: Arc::new(Mutex::new(VecDeque::new())),
            time_chunks_passed: Arc::new(Mutex::new(start_time)),
            timer,
            buffering_done: Arc::new(AtomicBool::new(false)),
            final_duration: Arc::new(Mutex::new(None)),
            last_meter_time: 0.0,
            append_timing: Arc::new(Mutex::new((Instant::now(), 0.0, 0, 0.0))),
            resuming_gate_started_at: None,
        }
    }

    /// Recoverable poison policy: queued chunk lengths are rebuildable runtime bookkeeping.
    pub(super) fn lock_chunk_lengths_recoverable(&self) -> MutexGuard<'_, VecDeque<f64>> {
        lock_recoverable(
            &self.chunk_lengths,
            "playback chunk lengths",
            "chunk length bookkeeping can be rebuilt from future appends",
        )
    }

    /// Recoverable poison policy: accumulated chunk time is scalar bookkeeping.
    pub(super) fn lock_time_chunks_passed_recoverable(&self) -> MutexGuard<'_, f64> {
        lock_recoverable(
            &self.time_chunks_passed,
            "playback chunk time",
            "accumulated chunk time is scalar bookkeeping",
        )
    }

    /// Recoverable poison policy: the timer is disposable runtime coordination state.
    pub(super) fn lock_timer_recoverable(&self) -> MutexGuard<'_, timer::Timer> {
        lock_recoverable(
            &self.timer,
            "playback timer",
            "the timer is disposable runtime coordination state",
        )
    }

    /// Recoverable poison policy: final drain duration is derived runtime bookkeeping.
    pub(super) fn lock_final_duration_recoverable(&self) -> MutexGuard<'_, Option<f64>> {
        lock_recoverable(
            &self.final_duration,
            "playback final duration",
            "final drain duration is derived runtime bookkeeping",
        )
    }

    /// Recoverable poison policy: append timing metrics are derived telemetry.
    pub(super) fn lock_append_timing_recoverable(
        &self,
    ) -> MutexGuard<'_, (Instant, f64, u64, f64)> {
        lock_recoverable(
            &self.append_timing,
            "playback append timing",
            "append timing is derived telemetry that can resume from the inner value",
        )
    }
}

// Run the playback worker loop for a single generation (`playback_id`).
//
// # Arguments
//
// * `ctx` - Captured shared state and handles for this run.
// * `playback_id` - Generation ID used to invalidate stale workers.
// * `ts` - Optional start timestamp in seconds.
pub(in crate::playback::player::runtime) fn run_playback_thread(
    ctx: ThreadContext,
    playback_id: u64,
    ts: Option<f64>,
) {
    let _thread_guard = PlaybackThreadGuard::new(ctx.playback_thread_exists.clone());
    let start_time = ts.unwrap_or(0.0);
    if let Some(elapsed_ms) = play_trace_elapsed_ms(&ctx) {
        debug!(
            "play trace: playback worker start playback_id={} ts={:.3} +{}ms",
            playback_id, start_time, elapsed_ms
        );
    }

    let mut engine = PlayerEngine::new(
        ctx.prot.clone(),
        PlayerEngineConfig {
            abort_option: Some(ctx.abort.clone()),
            start_time,
            buffer_settings: ctx.buffer_settings.clone(),
            effects: ctx.effects.clone(),
            dsp_metrics: ctx.dsp_metrics.clone(),
            effects_reset: ctx.effects_reset.clone(),
            inline_effects_update: ctx.inline_effects_update.clone(),
            inline_track_mix_updates: ctx.inline_track_mix_updates.clone(),
        },
    );

    initialize_sink(&ctx, &ctx.output_mixer);
    if let Some(elapsed_ms) = play_trace_elapsed_ms(&ctx) {
        debug!("play trace: sink initialized +{}ms", elapsed_ms);
    }
    set_duration_from_engine(&ctx, &engine);
    set_start_time(&ctx, start_time);
    append_startup_silence(&ctx);

    let mut loop_state = LoopState::new(start_time);

    let receiver = engine.start_receiver();
    if let Some(elapsed_ms) = play_trace_elapsed_ms(&ctx) {
        debug!("play trace: engine receiver started +{}ms", elapsed_ms);
    }
    run_engine_receive_loop(&ctx, &mut loop_state, playback_id, receiver);
    #[cfg(feature = "debug")]
    log::info!("engine reception loop finished");

    mark_buffering_complete(&ctx, &loop_state);

    #[cfg(feature = "debug")]
    log_drain_loop_start(&ctx, &loop_state);

    let drain_completed = run_drain_loop(&ctx, &mut loop_state, &engine);

    #[cfg(feature = "debug")]
    log::info!("finished drain loop");

    if drain_completed {
        apply_end_of_stream_action(&ctx, &loop_state);
    }
}

fn run_engine_receive_loop(
    ctx: &ThreadContext,
    loop_state: &mut LoopState,
    playback_id: u64,
    receiver: std::sync::mpsc::Receiver<(SamplesBuffer, f64)>,
) {
    let mut logged_first_engine_chunk = false;
    loop {
        if ctx.abort.load(Ordering::SeqCst) {
            break;
        }
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(chunk) => {
                if !logged_first_engine_chunk {
                    logged_first_engine_chunk = true;
                    if let Some(elapsed_ms) = play_trace_elapsed_ms(ctx) {
                        debug!("play trace: first engine chunk received +{}ms", elapsed_ms);
                    }
                }
                update_sink(ctx, loop_state, playback_id, chunk);
                if ctx.abort.load(Ordering::SeqCst) {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                update_chunk_lengths(ctx, loop_state);
                if !check_runtime_state(ctx, loop_state) {
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

// Snapshot the total engine duration into shared player state.
//
// # Arguments
//
// * `ctx` - Shared worker context containing duration state.
// * `engine` - Active engine instance for this playback run.
fn set_duration_from_engine(ctx: &ThreadContext, engine: &PlayerEngine) {
    let mut duration = ctx.lock_duration_recoverable();
    *duration = engine.get_duration();
}

// Initialize shared playback time to the selected start position.
//
// # Arguments
//
// * `ctx` - Shared worker context containing playback time state.
// * `start_time` - Start position in seconds.
fn set_start_time(ctx: &ThreadContext, start_time: f64) {
    let mut time_passed = ctx.lock_time_passed_recoverable();
    *time_passed = start_time;
}

#[cfg(test)]
mod tests {
    use super::LoopState;

    #[test]
    fn loop_state_initializes_with_start_time() {
        let state = LoopState::new(3.25);
        assert_eq!(state.start_time, 3.25);
        assert!(state.startup_fade_pending);
        assert_eq!(*state.time_chunks_passed.lock().unwrap(), 3.25);
        assert!(!state
            .buffering_done
            .load(std::sync::atomic::Ordering::Relaxed));
        assert!(state.final_duration.lock().unwrap().is_none());
    }
}
