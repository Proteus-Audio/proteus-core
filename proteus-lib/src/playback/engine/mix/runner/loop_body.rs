//! Per-iteration helper functions for the mix-thread loop.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rodio::buffer::SamplesBuffer;
use log::{debug, info, warn};

use crate::container::info::Info;
use crate::container::prot::Prot;
use crate::dsp::effects::{convolution_reverb, AudioEffect, EffectContext};
use crate::playback::engine::{DspChainMetrics, InlineEffectsUpdate, InlineTrackMixUpdate};
#[cfg(feature = "debug")]
use crate::logging::pivot_buffer_trace::pivot_buffer;

use super::super::buffer_mixer::{BufferMixer, DecodeBackpressure, SourceKey};
use super::super::decoder_events::DecodeWorkerEvent;
use super::super::effects::run_effect_chain;
use super::super::output_stage;
use super::super::types::{ActiveInlineTransition, MixThreadArgs};
use super::decode::{spawn_container_decode_worker, spawn_file_decode_worker, DecodeWorkerJoinGuard};

pub(super) const MAX_EFFECT_DRAIN_PASSES: usize = 1024;
pub(super) const DRAIN_SILENCE_EPSILON: f32 = 1.0e-6;
pub(super) const DRAIN_SILENT_PASSES_TO_STOP: usize = 2;

pub(super) struct MixLoopState {
    pub(super) abort: Arc<AtomicBool>,
    packet_rx: mpsc::Receiver<DecodeWorkerEvent>,
    buffer_mixer: BufferMixer,
    decode_backpressure: Arc<DecodeBackpressure>,
    effects: Arc<Mutex<Vec<AudioEffect>>>,
    effect_context: EffectContext,
    sender: mpsc::SyncSender<(SamplesBuffer, f64)>,
    buffer_notify: Arc<Condvar>,
    audio_info: Info,
    dsp_metrics: Arc<Mutex<DspChainMetrics>>,
    inline_track_mix_updates: Arc<Mutex<Vec<InlineTrackMixUpdate>>>,
    inline_effects_update: Arc<Mutex<Option<InlineEffectsUpdate>>>,
    effects_reset: Arc<AtomicU64>,
    prot: Arc<Mutex<Prot>>,
    finished_tracks: Arc<Mutex<Vec<u16>>>,
    convolution_batch_samples: usize,
    start_samples: usize,
    min_mix_samples: usize,
    started: bool,
    last_effects_reset: u64,
    active_inline_transition: Option<ActiveInlineTransition>,
    pending_mix_samples: Vec<f32>,
    effect_drain_passes: usize,
    effect_drain_silent_passes: usize,
    running_count: usize,
    logged_first_packet_drain: bool,
    logged_first_packet_route: bool,
    logged_start_gate: bool,
    logged_first_take_samples: bool,
    logged_first_output_send: bool,
    decode_workers: DecodeWorkerJoinGuard,
    #[cfg(feature = "debug")]
    alpha: f64,
    #[cfg(feature = "debug")]
    avg_overrun_ms: f64,
    #[cfg(feature = "debug")]
    max_overrun_ms: f64,
    #[cfg(feature = "debug")]
    avg_chain_ksps: f64,
    #[cfg(feature = "debug")]
    min_chain_ksps: f64,
    #[cfg(feature = "debug")]
    max_chain_ksps: f64,
}

pub(super) fn setup_mix_state(
    args: MixThreadArgs,
    sender: mpsc::SyncSender<(SamplesBuffer, f64)>,
    startup_trace: Instant,
) -> Option<MixLoopState> {
    let MixThreadArgs {
        audio_info, buffer_notify, effects_reset, inline_effects_update,
        inline_track_mix_updates, finished_tracks, prot, abort, start_time,
        buffer_settings, effects, dsp_metrics, ..
    } = args;
    info!("mix startup trace: thread start");
    let (instance_plan, container_path, effect_context, track_mix_settings_by_slot) = {
        let p = prot.lock().unwrap();
        (
            p.build_runtime_instance_plan(start_time),
            p.get_container_path(),
            EffectContext {
                sample_rate: p.info.sample_rate,
                channels: p.info.channels as usize,
                container_path: p.get_container_path(),
                impulse_response_spec: p.get_impulse_response_spec(),
                impulse_response_tail_db: p.get_impulse_response_tail_db().unwrap_or(-60.0),
            },
            p.get_track_mix_settings(),
        )
    };
    info!("mix startup trace: runtime plan built in {}ms (instances={})", startup_trace.elapsed().as_millis(), instance_plan.instances.len());
    if instance_plan.instances.is_empty() { abort.store(true, Ordering::SeqCst); return None; }
    const MIN_MIX_MS: f32 = 30.0;
    let start_buffer_ms = buffer_settings.lock().unwrap().start_buffer_ms;
    let start_samples = ((audio_info.sample_rate as f32 * start_buffer_ms) / 1000.0) as usize * audio_info.channels as usize;
    let mut min_mix_samples = (((audio_info.sample_rate as f32 * MIN_MIX_MS) / 1000.0) as usize).max(1) * audio_info.channels as usize;
    let has_convolution = effects.lock().unwrap().iter().any(|e| matches!(e, AudioEffect::ConvolutionReverb(e) if e.enabled));
    let convolution_batch_samples = if has_convolution { convolution_reverb::preferred_batch_samples(audio_info.channels.max(1) as usize) } else { 0 };
    if has_convolution && convolution_batch_samples > 0 { min_mix_samples = min_mix_samples.div_ceil(convolution_batch_samples) * convolution_batch_samples; }
    let mut track_mix_by_logical: HashMap<usize, (f32, f32)> = HashMap::new();
    for instance in instance_plan.instances.iter() {
        track_mix_by_logical.entry(instance.logical_track_index).or_insert_with(|| {
            track_mix_settings_by_slot.get(&(instance.slot_index as u16)).copied().unwrap_or((1.0, 0.0))
        });
    }
    let track_buffer_size = ((audio_info.sample_rate as usize * 10) * audio_info.channels.max(1) as usize).max(start_samples * 2);
    let buffer_mixer = BufferMixer::new(instance_plan, audio_info.sample_rate, audio_info.channels.max(1) as usize, track_buffer_size, track_mix_by_logical, min_mix_samples);
    info!("mix startup trace: buffer_mixer ready in {}ms (track_buffer_size={} min_mix_samples={} start_samples={})", startup_trace.elapsed().as_millis(), track_buffer_size, min_mix_samples, start_samples);
    let decode_backpressure = buffer_mixer.decode_backpressure();
    let (packet_rx, decode_workers) = spawn_mix_decode_workers(
        &buffer_mixer, container_path, start_time, audio_info.channels as u8,
        start_samples.max(min_mix_samples), &decode_backpressure, &abort, startup_trace,
    );
    if min_mix_samples > 0 {
        for effect in effects.lock().unwrap().iter_mut() { effect.warm_up(&effect_context); }
    }
    info!("mix startup trace: effect warmup complete in {}ms (warmup_samples={})", startup_trace.elapsed().as_millis(), min_mix_samples);
    let last_effects_reset = effects_reset.load(Ordering::SeqCst);
    Some(MixLoopState {
        abort, packet_rx, buffer_mixer, decode_backpressure, effects, effect_context,
        sender, buffer_notify, audio_info, dsp_metrics, inline_track_mix_updates,
        inline_effects_update, effects_reset, prot, finished_tracks,
        convolution_batch_samples, start_samples, min_mix_samples,
        started: start_samples == 0, last_effects_reset,
        active_inline_transition: None, pending_mix_samples: Vec::new(),
        effect_drain_passes: 0, effect_drain_silent_passes: 0, running_count: 0,
        logged_first_packet_drain: false, logged_first_packet_route: false,
        logged_start_gate: false, logged_first_take_samples: false,
        logged_first_output_send: false, decode_workers,
        #[cfg(feature = "debug")]
        alpha: 0.1,
        #[cfg(feature = "debug")]
        avg_overrun_ms: 0.0,
        #[cfg(feature = "debug")]
        max_overrun_ms: 0.0,
        #[cfg(feature = "debug")]
        avg_chain_ksps: 0.0,
        #[cfg(feature = "debug")]
        min_chain_ksps: f64::INFINITY,
        #[cfg(feature = "debug")]
        max_chain_ksps: 0.0,
    })
}

fn spawn_mix_decode_workers(
    buffer_mixer: &BufferMixer,
    container_path: Option<String>,
    start_time: f64,
    channels: u8,
    startup_gate_samples: usize,
    decode_backpressure: &Arc<DecodeBackpressure>,
    abort: &Arc<AtomicBool>,
    startup_trace: Instant,
) -> (mpsc::Receiver<DecodeWorkerEvent>, DecodeWorkerJoinGuard) {
    let (packet_tx, packet_rx) = mpsc::sync_channel::<DecodeWorkerEvent>(64);
    let mut decode_workers = DecodeWorkerJoinGuard::default();
    let (mut track_ids, mut file_paths): (HashSet<_>, HashSet<_>) = (HashSet::new(), HashSet::new());
    for source in buffer_mixer.sources() {
        match source {
            SourceKey::TrackId(id) => { track_ids.insert(id); }
            SourceKey::FilePath(p) => { file_paths.insert(p); }
        }
    }
    if !file_paths.is_empty() && startup_gate_samples > 0 {
        decode_backpressure.enable_startup_priority(startup_gate_samples);
    }
    if !track_ids.is_empty() {
        if let Some(path) = container_path {
            decode_workers.push(spawn_container_decode_worker(path, track_ids.into_iter().collect(), start_time, channels, packet_tx.clone(), abort.clone(), decode_backpressure.clone()));
        }
    }
    for path in file_paths {
        decode_workers.push(spawn_file_decode_worker(path, start_time, channels, packet_tx.clone(), abort.clone(), decode_backpressure.clone()));
    }
    drop(packet_tx);
    let container_count = if buffer_mixer.sources().iter().any(|s| matches!(s, SourceKey::TrackId(_))) { 1 } else { 0 };
    let file_count = buffer_mixer.sources().iter().filter(|s| matches!(s, SourceKey::FilePath(_))).count();
    info!("mix startup trace: decode workers spawned in {}ms (container_sources={} file_sources={})", startup_trace.elapsed().as_millis(), container_count, file_count);
    (packet_rx, decode_workers)
}

pub(super) fn run_mix_loop(state: &mut MixLoopState, startup_trace: Instant) {
    loop {
        if state.abort.load(Ordering::SeqCst) { break; }
        drain_decode_events(&state.packet_rx, &mut state.buffer_mixer, startup_trace, &mut state.logged_first_packet_drain, &mut state.logged_first_packet_route);
        apply_inline_track_mix_updates(&state.inline_track_mix_updates, &mut state.buffer_mixer);
        apply_effect_runtime_updates(&state.effects_reset, &mut state.last_effects_reset, &state.effects, &mut state.active_inline_transition, &state.inline_effects_update, &state.prot, &state.audio_info, &mut state.effect_context);
        if !state.started {
            if state.buffer_mixer.mix_ready_with_min_samples(state.start_samples.max(state.min_mix_samples)) {
                state.started = true;
                state.decode_backpressure.disable_startup_priority();
                if !state.logged_start_gate {
                    state.logged_start_gate = true;
                    info!("mix startup trace: start gate satisfied at {}ms", startup_trace.elapsed().as_millis());
                }
            } else {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
        }
        if let Some(samples) = take_next_samples(state, startup_trace) {
            if !process_and_send_samples(samples, state, startup_trace) { break; }
        } else if state.buffer_mixer.mix_finished() {
            if !drain_effect_tail(state) { break; }
        } else {
            thread::sleep(Duration::from_millis(2));
        }
    }
}

fn take_next_samples(state: &mut MixLoopState, startup_trace: Instant) -> Option<Vec<f32>> {
    let batch = state.convolution_batch_samples;
    if batch > 0 && state.pending_mix_samples.len() >= batch {
        return Some(state.pending_mix_samples.drain(0..batch).collect());
    }
    if let Some(samples) = state.buffer_mixer.take_samples() {
        if !state.logged_first_take_samples {
            state.logged_first_take_samples = true;
            info!("mix startup trace: first take_samples at {}ms (samples={})", startup_trace.elapsed().as_millis(), samples.len());
        }
        if batch > 0 {
            state.pending_mix_samples.extend_from_slice(&samples);
            if state.pending_mix_samples.len() >= batch {
                return Some(state.pending_mix_samples.drain(0..batch).collect());
            }
        } else {
            return Some(samples);
        }
    }
    if state.buffer_mixer.mix_finished() && !state.pending_mix_samples.is_empty() {
        let missing = batch.saturating_sub(state.pending_mix_samples.len());
        state.pending_mix_samples.extend(std::iter::repeat(0.0).take(missing));
        return Some(std::mem::take(&mut state.pending_mix_samples));
    }
    None
}

fn process_and_send_samples(samples: Vec<f32>, state: &mut MixLoopState, startup_trace: Instant) -> bool {
    state.running_count += samples.len();
    debug!("Processed {} samples so far!", state.running_count);
    if samples.len() < state.convolution_batch_samples {
        warn!("Only processing {} samples! (Convolution wants {})", samples.len(), state.convolution_batch_samples);
    }
    #[cfg(feature = "debug")]
    let audio_time_ms = if state.audio_info.channels > 0 && state.audio_info.sample_rate > 0 {
        (samples.len() as f64 / state.audio_info.channels as f64 / state.audio_info.sample_rate as f64) * 1000.0
    } else { 0.0 };
    #[cfg(feature = "debug")]
    let dsp_start = Instant::now();
    let processed = if let Some(transition) = state.active_inline_transition.as_mut() {
        let old_out = run_effect_chain(&mut transition.old_effects, &samples, &state.effect_context, false);
        let new_out = run_effect_chain(&mut transition.new_effects, &samples, &state.effect_context, false);
        let len = old_out.len().max(new_out.len());
        let mut blended = Vec::with_capacity(len);
        for i in 0..len {
            let o = old_out.get(i).copied().unwrap_or(0.0);
            let n = new_out.get(i).copied().unwrap_or(0.0);
            let mix = if transition.total_samples == 0 { 1.0 } else {
                let done = transition.total_samples.saturating_sub(transition.remaining_samples);
                (done as f32 / transition.total_samples as f32).clamp(0.0, 1.0)
            };
            blended.push((o * (1.0 - mix)) + (n * mix));
        }
        transition.remaining_samples = transition.remaining_samples.saturating_sub(samples.len().max(1));
        if transition.remaining_samples == 0 {
            *state.effects.lock().unwrap() = transition.new_effects.clone();
            state.active_inline_transition = None;
        }
        blended
    } else {
        run_effect_chain(&mut state.effects.lock().unwrap(), &samples, &state.effect_context, false)
    };
    #[cfg(feature = "debug")]
    update_debug_metrics(state, dsp_start, audio_time_ms, processed.len());
    match output_stage::send_samples(&state.sender, state.audio_info.channels as u16, state.audio_info.sample_rate, processed) {
        output_stage::SendStatus::Sent => {
            if !state.logged_first_output_send {
                state.logged_first_output_send = true;
                info!("mix startup trace: first output chunk sent at {}ms (processed_samples={})", startup_trace.elapsed().as_millis(), samples.len());
            }
            state.buffer_notify.notify_all();
        }
        output_stage::SendStatus::Empty => {}
        output_stage::SendStatus::Disconnected => {
            state.abort.store(true, Ordering::SeqCst);
            return false;
        }
    }
    if let Ok(mut metrics) = state.dsp_metrics.lock() {
        metrics.track_key_count = state.buffer_mixer.instance_count();
        metrics.prot_key_count = state.buffer_mixer.logical_track_count();
        metrics.finished_track_count = state.buffer_mixer.finished_instance_count();
    }
    true
}

#[cfg(feature = "debug")]
fn update_debug_metrics(state: &mut MixLoopState, dsp_start: Instant, audio_time_ms: f64, processed_len: usize) {
    let dsp_time_ms = dsp_start.elapsed().as_secs_f64() * 1000.0;
    let overrun_ms = (dsp_time_ms - audio_time_ms).max(0.0);
    let chain_ksps = if dsp_time_ms > 0.0 { (processed_len as f64 / (dsp_time_ms / 1000.0)) / 1000.0 } else { 0.0 };
    state.avg_overrun_ms = if state.avg_overrun_ms == 0.0 { overrun_ms } else { (state.avg_overrun_ms * (1.0 - state.alpha)) + (overrun_ms * state.alpha) };
    state.avg_chain_ksps = if state.avg_chain_ksps == 0.0 { chain_ksps } else { (state.avg_chain_ksps * (1.0 - state.alpha)) + (chain_ksps * state.alpha) };
    if overrun_ms > 0.0 { state.max_overrun_ms = state.max_overrun_ms.max(overrun_ms); }
    if chain_ksps > 0.0 { state.min_chain_ksps = state.min_chain_ksps.min(chain_ksps); state.max_chain_ksps = state.max_chain_ksps.max(chain_ksps); }
    if let Ok(mut metrics) = state.dsp_metrics.lock() {
        metrics.overrun = dsp_time_ms > audio_time_ms;
        metrics.overrun_ms = overrun_ms;
        metrics.avg_overrun_ms = state.avg_overrun_ms;
        metrics.max_overrun_ms = state.max_overrun_ms;
        metrics.chain_ksps = chain_ksps;
        metrics.avg_chain_ksps = state.avg_chain_ksps;
        metrics.min_chain_ksps = if state.min_chain_ksps.is_finite() { state.min_chain_ksps } else { 0.0 };
        metrics.max_chain_ksps = state.max_chain_ksps;
    }
}

fn drain_effect_tail(state: &mut MixLoopState) -> bool {
    #[cfg(feature = "debug")]
    let _ = pivot_buffer();
    info!("Mix Finished!!! (in runner)");
    state.effect_drain_passes = state.effect_drain_passes.saturating_add(1);
    if state.effect_drain_passes > MAX_EFFECT_DRAIN_PASSES {
        warn!("effect drain stopped after {} passes to avoid infinite tail generation", MAX_EFFECT_DRAIN_PASSES);
        return false;
    }
    let drained = if let Some(transition) = state.active_inline_transition.as_mut() {
        let old_out = run_effect_chain(&mut transition.old_effects, &[], &state.effect_context, true);
        let new_out = run_effect_chain(&mut transition.new_effects, &[], &state.effect_context, true);
        let len = old_out.len().max(new_out.len());
        let mut blended = Vec::with_capacity(len);
        for i in 0..len {
            blended.push((old_out.get(i).copied().unwrap_or(0.0) + new_out.get(i).copied().unwrap_or(0.0)) * 0.5);
        }
        blended
    } else {
        run_effect_chain(&mut state.effects.lock().unwrap(), &[], &state.effect_context, true)
    };
    if drained.is_empty() { return false; }
    let max_abs = drained.iter().fold(0.0_f32, |acc, s| acc.max(s.abs()));
    if max_abs <= DRAIN_SILENCE_EPSILON {
        state.effect_drain_silent_passes = state.effect_drain_silent_passes.saturating_add(1);
    } else {
        state.effect_drain_silent_passes = 0;
    }
    if state.effect_drain_silent_passes >= DRAIN_SILENT_PASSES_TO_STOP {
        info!("effect drain stopped after consecutive silent drain passes");
        return false;
    }
    match output_stage::send_samples(&state.sender, state.audio_info.channels as u16, state.audio_info.sample_rate, drained) {
        output_stage::SendStatus::Sent => true,
        output_stage::SendStatus::Empty => false,
        output_stage::SendStatus::Disconnected => { state.abort.store(true, Ordering::SeqCst); false }
    }
}

pub(super) fn teardown_mix(state: MixLoopState) {
    {
        let mut finished = state.finished_tracks.lock().unwrap();
        finished.clear();
        for idx in 0..state.buffer_mixer.instance_count() {
            finished.push(idx as u16);
        }
    }
    // Destructure to control drop order: packet_rx must drop before decode_workers
    // so workers blocked on packet_tx.send(...) wake with SendError instead of deadlocking.
    let MixLoopState { decode_backpressure, packet_rx, decode_workers, .. } = state;
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
        let mut pending = inline_track_mix_updates.lock().unwrap();
        std::mem::take(&mut *pending)
    };
    for update in updates {
        buffer_mixer.set_track_mix_by_slot(update.slot_index, update.level, update.pan);
    }
}

/// Apply effect resets and inline effect transitions for the current loop iteration.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_effect_runtime_updates(
    effects_reset: &Arc<std::sync::atomic::AtomicU64>,
    last_effects_reset: &mut u64,
    effects: &Arc<Mutex<Vec<AudioEffect>>>,
    active_inline_transition: &mut Option<ActiveInlineTransition>,
    inline_effects_update: &Arc<Mutex<Option<crate::playback::engine::InlineEffectsUpdate>>>,
    prot_locked: &Arc<Mutex<crate::container::prot::Prot>>,
    audio_info: &crate::container::info::Info,
    effect_context: &mut EffectContext,
) {
    let current_reset = effects_reset.load(Ordering::SeqCst);
    if current_reset != *last_effects_reset {
        let mut effects_guard = effects.lock().unwrap();
        for effect in effects_guard.iter_mut() {
            effect.reset_state();
        }
        *active_inline_transition = None;
        inline_effects_update.lock().unwrap().take();
        *effect_context = {
            let prot = prot_locked.lock().unwrap();
            EffectContext {
                sample_rate: prot.info.sample_rate,
                channels: prot.info.channels as usize,
                container_path: prot.get_container_path(),
                impulse_response_spec: prot.get_impulse_response_spec(),
                impulse_response_tail_db: prot.get_impulse_response_tail_db().unwrap_or(-60.0),
            }
        };
        *last_effects_reset = current_reset;
    }

    if let Some(update) = inline_effects_update.lock().unwrap().take() {
        let transition_samples = ((update.transition_ms / 1000.0)
            * audio_info.sample_rate.max(1) as f32)
            .round() as usize
            * audio_info.channels.max(1) as usize;
        if transition_samples == 0 {
            let mut effects_guard = effects.lock().unwrap();
            *effects_guard = update.effects;
            for effect in effects_guard.iter_mut() {
                effect.warm_up(effect_context);
            }
            *active_inline_transition = None;
        } else {
            let old_effects = {
                let effects_guard = effects.lock().unwrap();
                effects_guard.clone()
            };
            let mut new_effects = update.effects;
            for effect in new_effects.iter_mut() {
                effect.warm_up(effect_context);
            }
            *active_inline_transition = Some(ActiveInlineTransition {
                old_effects,
                new_effects,
                total_samples: transition_samples,
                remaining_samples: transition_samples,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DRAIN_SILENCE_EPSILON, DRAIN_SILENT_PASSES_TO_STOP, MAX_EFFECT_DRAIN_PASSES};

    #[test]
    fn drain_constants_are_positive() {
        assert!(MAX_EFFECT_DRAIN_PASSES > 0);
        assert!(DRAIN_SILENT_PASSES_TO_STOP > 0);
        assert!(DRAIN_SILENCE_EPSILON > 0.0);
    }
}
