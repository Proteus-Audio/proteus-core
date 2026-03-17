//! Per-iteration helper functions for the mix-thread loop.

use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use log::{info, warn};

use super::super::buffer_mixer::BufferMixer;
use super::super::decoder_events::DecodeWorkerEvent;
use super::effects_runtime;
use super::state::MixLoopState;

pub(super) const MAX_EFFECT_DRAIN_PASSES: usize = 1024;
pub(super) const DRAIN_SILENCE_EPSILON: f32 = 1.0e-6;
pub(super) const DRAIN_SILENT_PASSES_TO_STOP: usize = 2;

pub(super) fn run_mix_loop(state: &mut MixLoopState, startup_trace: Instant) {
    loop {
        if state.abort.load(Ordering::SeqCst) {
            break;
        }
        drain_decode_events(
            &state.packet_rx,
            &mut state.buffer_mixer,
            startup_trace,
            &mut state.logged_first_packet_drain,
            &mut state.logged_first_packet_route,
        );
        apply_inline_track_mix_updates(&state.inline_track_mix_updates, &mut state.buffer_mixer);
        effects_runtime::apply_effect_runtime_updates(state);
        if !state.started {
            if state
                .buffer_mixer
                .mix_ready_with_min_samples(state.start_samples.max(state.min_mix_samples))
            {
                state.started = true;
                state.decode_backpressure.disable_startup_priority();
                if !state.logged_start_gate {
                    state.logged_start_gate = true;
                    info!(
                        "mix startup trace: start gate satisfied at {}ms",
                        startup_trace.elapsed().as_millis()
                    );
                }
            } else {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
        }
        if let Some(samples) = take_next_samples(state, startup_trace) {
            if !effects_runtime::process_and_send_samples(samples, state, startup_trace) {
                break;
            }
        } else if state.buffer_mixer.mix_finished() {
            if !effects_runtime::drain_effect_tail(state) {
                break;
            }
        } else {
            thread::sleep(Duration::from_millis(2));
        }
    }
}

fn take_next_samples(state: &mut MixLoopState, startup_trace: Instant) -> Option<Vec<f32>> {
    let batch = state.convolution_batch_samples;
    if batch > 0 && state.pending_mix_samples.len() >= batch {
        return Some(state.pending_mix_samples.pop_chunk(batch));
    }
    if let Some(samples) = state.buffer_mixer.take_samples() {
        if !state.logged_first_take_samples {
            state.logged_first_take_samples = true;
            info!(
                "mix startup trace: first take_samples at {}ms (samples={})",
                startup_trace.elapsed().as_millis(),
                samples.len()
            );
        }
        if batch > 0 {
            state.pending_mix_samples.push_interleaved(&samples);
            if state.pending_mix_samples.len() >= batch {
                return Some(state.pending_mix_samples.pop_chunk(batch));
            }
        } else {
            return Some(samples);
        }
    }
    if state.buffer_mixer.mix_finished() && !state.pending_mix_samples.is_empty() {
        let remaining = state.pending_mix_samples.len();
        let missing = batch.saturating_sub(remaining);
        let zeros = vec![0.0_f32; missing];
        state.pending_mix_samples.push_interleaved(&zeros);
        return Some(state.pending_mix_samples.pop_chunk(batch));
    }
    None
}

pub(super) fn teardown_mix(state: MixLoopState) {
    {
        let mut finished = state.lock_finished_tracks_recoverable();
        finished.clear();
        for idx in 0..state.buffer_mixer.instance_count() {
            finished.push(idx as u16);
        }
    }
    // Destructure to control drop order: packet_rx must drop before decode_workers
    // so workers blocked on packet_tx.send(...) wake with SendError instead of deadlocking.
    let MixLoopState {
        decode_backpressure,
        packet_rx,
        decode_workers,
        ..
    } = state;
    decode_backpressure.shutdown();
    drop(packet_rx);
    drop(decode_workers);
}

/// Drain all pending decode worker events from the channel into the buffer mixer.
pub(super) fn drain_decode_events(
    packet_rx: &mpsc::Receiver<DecodeWorkerEvent>,
    buffer_mixer: &mut BufferMixer,
    startup_trace: Instant,
    logged_first_packet_drain: &mut bool,
    logged_first_packet_route: &mut bool,
) {
    while let Ok(event) = packet_rx.try_recv() {
        if !*logged_first_packet_drain {
            info!(
                "mix startup trace: first packet dequeued from decode channel at {}ms",
                startup_trace.elapsed().as_millis()
            );
            *logged_first_packet_drain = true;
        }
        match event {
            DecodeWorkerEvent::Packet(packet) => {
                if !*logged_first_packet_route {
                    info!(
                        "mix startup trace: first packet route start at {}ms (source={:?} ts={:.6} samples={})",
                        startup_trace.elapsed().as_millis(),
                        packet.source_key,
                        packet.packet_ts,
                        packet.samples.len()
                    );
                    *logged_first_packet_route = true;
                }
                let _decision =
                    buffer_mixer.route_packet(&packet.samples, packet.source_key, packet.packet_ts);
            }
            DecodeWorkerEvent::SourceFinished { source_key } => {
                buffer_mixer.signal_finish(&source_key);
            }
            DecodeWorkerEvent::SourceError {
                source_key,
                recoverable,
                message,
            } => {
                if recoverable {
                    warn!(
                        "decode worker recoverable error: source={:?} {}",
                        source_key, message
                    );
                } else {
                    warn!(
                        "decode worker terminal error: source={:?} {}",
                        source_key, message
                    );
                    buffer_mixer.signal_finish(&source_key);
                }
            }
            DecodeWorkerEvent::StreamExhausted => {
                buffer_mixer.signal_finish_all();
            }
        }
    }
}

/// Flush pending inline track mix updates into the buffer mixer.
pub(super) fn apply_inline_track_mix_updates(
    inline_track_mix_updates: &Arc<Mutex<Vec<crate::playback::engine::InlineTrackMixUpdate>>>,
    buffer_mixer: &mut BufferMixer,
) {
    let updates = {
        let mut pending = crate::playback::mutex_policy::lock_recoverable(
            inline_track_mix_updates,
            "mix runtime inline track mix updates",
            "pending inline track-mix updates are a disposable queue",
        );
        std::mem::take(&mut *pending)
    };
    for update in updates {
        buffer_mixer.set_track_mix_by_slot(update.slot_index, update.level, update.pan);
    }
}

/// Apply effect resets and inline effect transitions for the current loop iteration.
#[cfg(test)]
mod tests;
