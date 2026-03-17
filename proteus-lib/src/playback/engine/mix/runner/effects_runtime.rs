//! Effect-chain processing, draining, and runtime update helpers.
//!
//! The mix thread owns a local copy of the effect chain (`local_effects`) and
//! runs all DSP processing on it without holding the shared effects mutex.
//! Control-path settings changes arrive through a lightweight command queue
//! that is drained at chunk boundaries.

use std::sync::atomic::Ordering;
use std::time::Instant;

use log::{debug, info, warn};

use crate::dsp::effects::EffectContext;
#[cfg(feature = "debug")]
use crate::logging::pivot_buffer_trace::pivot_buffer;

use super::super::effects::run_effect_chain;
use super::super::output_stage;
use super::super::types::EffectSettingsCommand;
use super::loop_body::{
    DRAIN_SILENCE_EPSILON, DRAIN_SILENT_PASSES_TO_STOP, MAX_EFFECT_DRAIN_PASSES,
};
use super::state::MixLoopState;

pub(super) fn process_and_send_samples(
    samples: Vec<f32>,
    state: &mut MixLoopState,
    startup_trace: Instant,
) -> bool {
    state.running_count += samples.len();
    debug!("processed {} samples so far", state.running_count);
    if samples.len() < state.convolution_batch_samples {
        warn!(
            "Only processing {} samples! (Convolution wants {})",
            samples.len(),
            state.convolution_batch_samples
        );
    }
    #[cfg(feature = "debug")]
    let audio_time_ms = if state.audio_info.channels > 0 && state.audio_info.sample_rate > 0 {
        (samples.len() as f64
            / state.audio_info.channels as f64
            / state.audio_info.sample_rate as f64)
            * 1000.0
    } else {
        0.0
    };
    #[cfg(feature = "debug")]
    let dsp_start = Instant::now();
    let processed = process_effects(samples.as_slice(), state);
    #[cfg(feature = "debug")]
    update_debug_metrics(state, dsp_start, audio_time_ms, processed.len());
    match output_stage::send_samples(
        &state.sender,
        state.audio_info.channels as u16,
        state.audio_info.sample_rate,
        processed,
    ) {
        output_stage::SendStatus::Sent => {
            if !state.logged_first_output_send {
                state.logged_first_output_send = true;
                info!(
                    "mix startup trace: first output chunk sent at {}ms (processed_samples={})",
                    startup_trace.elapsed().as_millis(),
                    samples.len()
                );
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

fn process_effects(samples: &[f32], state: &mut MixLoopState) -> Vec<f32> {
    let result = if let Some(transition) = state.active_inline_transition.as_mut() {
        let old_out = run_effect_chain(
            &mut transition.old_effects,
            samples,
            &state.effect_context,
            false,
        );
        let new_out = run_effect_chain(
            &mut transition.new_effects,
            samples,
            &state.effect_context,
            false,
        );
        let len = old_out.len().max(new_out.len());
        let mut blended = Vec::with_capacity(len);
        for i in 0..len {
            let o = old_out.get(i).copied().unwrap_or(0.0);
            let n = new_out.get(i).copied().unwrap_or(0.0);
            let mix = if transition.total_samples == 0 {
                1.0
            } else {
                let done = transition
                    .total_samples
                    .saturating_sub(transition.remaining_samples);
                (done as f32 / transition.total_samples as f32).clamp(0.0, 1.0)
            };
            blended.push((o * (1.0 - mix)) + (n * mix));
        }
        transition.remaining_samples = transition
            .remaining_samples
            .saturating_sub(samples.len().max(1));
        blended
    } else {
        // DSP runs on the mix-thread-owned local chain — no mutex held.
        run_effect_chain(
            &mut state.local_effects,
            samples,
            &state.effect_context,
            false,
        )
    };

    // Finalize transition: adopt new effects as the local chain and sync shared.
    if state
        .active_inline_transition
        .as_ref()
        .is_some_and(|t| t.remaining_samples == 0)
    {
        if let Some(transition) = state.active_inline_transition.take() {
            let completed = transition.new_effects;
            // Brief lock: update shared for control-path reads.
            *state.effects.lock().unwrap_or_else(|_| {
                panic!("effects lock poisoned — a thread panicked while holding it")
            }) = completed.clone();
            state.local_effects = completed;
        }
    }

    result
}

#[cfg(feature = "debug")]
fn update_debug_metrics(
    state: &mut MixLoopState,
    dsp_start: Instant,
    audio_time_ms: f64,
    processed_len: usize,
) {
    let dsp_time_ms = dsp_start.elapsed().as_secs_f64() * 1000.0;
    let overrun_ms = (dsp_time_ms - audio_time_ms).max(0.0);
    let chain_ksps = if dsp_time_ms > 0.0 {
        (processed_len as f64 / (dsp_time_ms / 1000.0)) / 1000.0
    } else {
        0.0
    };
    state.avg_overrun_ms = if state.avg_overrun_ms == 0.0 {
        overrun_ms
    } else {
        (state.avg_overrun_ms * (1.0 - state.alpha)) + (overrun_ms * state.alpha)
    };
    state.avg_chain_ksps = if state.avg_chain_ksps == 0.0 {
        chain_ksps
    } else {
        (state.avg_chain_ksps * (1.0 - state.alpha)) + (chain_ksps * state.alpha)
    };
    if overrun_ms > 0.0 {
        state.max_overrun_ms = state.max_overrun_ms.max(overrun_ms);
    }
    if chain_ksps > 0.0 {
        state.min_chain_ksps = state.min_chain_ksps.min(chain_ksps);
        state.max_chain_ksps = state.max_chain_ksps.max(chain_ksps);
    }
    if let Ok(mut metrics) = state.dsp_metrics.lock() {
        metrics.overrun = dsp_time_ms > audio_time_ms;
        metrics.overrun_ms = overrun_ms;
        metrics.avg_overrun_ms = state.avg_overrun_ms;
        metrics.max_overrun_ms = state.max_overrun_ms;
        metrics.chain_ksps = chain_ksps;
        metrics.avg_chain_ksps = state.avg_chain_ksps;
        metrics.min_chain_ksps = if state.min_chain_ksps.is_finite() {
            state.min_chain_ksps
        } else {
            0.0
        };
        metrics.max_chain_ksps = state.max_chain_ksps;
    }
}

pub(super) fn drain_effect_tail(state: &mut MixLoopState) -> bool {
    #[cfg(feature = "debug")]
    let _ = pivot_buffer();
    info!("mix finished in runner");
    state.effect_drain_passes = state.effect_drain_passes.saturating_add(1);
    if state.effect_drain_passes > MAX_EFFECT_DRAIN_PASSES {
        warn!(
            "effect drain stopped after {} passes to avoid infinite tail generation",
            MAX_EFFECT_DRAIN_PASSES
        );
        return false;
    }

    let drained = if let Some(transition) = state.active_inline_transition.as_mut() {
        let old_out = run_effect_chain(
            &mut transition.old_effects,
            &[],
            &state.effect_context,
            true,
        );
        let new_out = run_effect_chain(
            &mut transition.new_effects,
            &[],
            &state.effect_context,
            true,
        );
        let len = old_out.len().max(new_out.len());
        let mut blended = Vec::with_capacity(len);
        for i in 0..len {
            blended.push(
                (old_out.get(i).copied().unwrap_or(0.0) + new_out.get(i).copied().unwrap_or(0.0))
                    * 0.5,
            );
        }
        blended
    } else {
        // Drain runs on the local chain — no mutex held.
        run_effect_chain(&mut state.local_effects, &[], &state.effect_context, true)
    };
    if drained.is_empty() {
        return false;
    }

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

    match output_stage::send_samples(
        &state.sender,
        state.audio_info.channels as u16,
        state.audio_info.sample_rate,
        drained,
    ) {
        output_stage::SendStatus::Sent => true,
        output_stage::SendStatus::Empty => false,
        output_stage::SendStatus::Disconnected => {
            state.abort.store(true, Ordering::SeqCst);
            false
        }
    }
}

pub(super) fn apply_effect_runtime_updates(state: &mut MixLoopState) {
    // Drain incremental settings commands from the control path.
    drain_effect_settings_commands(state);

    let current_reset = state.effects_reset.load(Ordering::SeqCst);
    if current_reset != state.last_effects_reset {
        // Full reset: clone the new chain from shared into local.
        state.local_effects = state
            .effects
            .lock()
            .unwrap_or_else(|_| {
                panic!("effects lock poisoned — a thread panicked while holding it")
            })
            .clone();
        for effect in state.local_effects.iter_mut() {
            effect.reset_state();
        }
        state.active_inline_transition = None;
        state
            .inline_effects_update
            .lock()
            .unwrap_or_else(|_| {
                panic!("inline effects update lock poisoned — a thread panicked while holding it")
            })
            .take();
        state.effect_context = rebuild_effect_context(&state.prot);
        state.last_effects_reset = current_reset;
    }

    if let Some(update) = state
        .inline_effects_update
        .lock()
        .unwrap_or_else(|_| {
            panic!("inline effects update lock poisoned — a thread panicked while holding it")
        })
        .take()
    {
        let transition_samples = ((update.transition_ms / 1000.0)
            * state.audio_info.sample_rate.max(1) as f32)
            .round() as usize
            * state.audio_info.channels.max(1) as usize;
        if transition_samples == 0 {
            // Instant replacement: adopt new chain as local, sync shared.
            state.local_effects = update.effects;
            for effect in state.local_effects.iter_mut() {
                effect.warm_up(&state.effect_context);
            }
            *state.effects.lock().unwrap_or_else(|_| {
                panic!("effects lock poisoned — a thread panicked while holding it")
            }) = state.local_effects.clone();
            state.active_inline_transition = None;
        } else {
            // Crossfade transition: snapshot local chain as old, warm up new.
            let old_effects = state.local_effects.clone();
            let mut new_effects = update.effects;
            for effect in new_effects.iter_mut() {
                effect.warm_up(&state.effect_context);
            }
            state.active_inline_transition = Some(
                crate::playback::engine::mix::types::ActiveInlineTransition {
                    old_effects,
                    new_effects,
                    total_samples: transition_samples,
                    remaining_samples: transition_samples,
                },
            );
        }
    }
}

/// Drain queued effect settings commands and apply them to the local chain.
fn drain_effect_settings_commands(state: &mut MixLoopState) {
    let commands = {
        let mut pending = state.effect_settings_commands.lock().unwrap_or_else(|_| {
            panic!("effect settings commands lock poisoned — a thread panicked while holding it")
        });
        if pending.is_empty() {
            return;
        }
        std::mem::take(&mut *pending)
    };
    for command in commands {
        match command {
            EffectSettingsCommand::SetReverbEnabled(enabled) => {
                for effect in state.local_effects.iter_mut() {
                    if let Some(e) = effect.as_convolution_reverb_mut() {
                        e.enabled = enabled;
                    }
                    if let Some(e) = effect.as_delay_reverb_mut() {
                        e.enabled = enabled;
                    }
                }
            }
            EffectSettingsCommand::SetReverbMix(dry_wet) => {
                let clamped = dry_wet.clamp(0.0, 1.0);
                for effect in state.local_effects.iter_mut() {
                    if let Some(e) = effect.as_convolution_reverb_mut() {
                        e.dry_wet = clamped;
                    }
                    if let Some(e) = effect.as_delay_reverb_mut() {
                        e.mix = clamped;
                    }
                    if let Some(e) = effect.as_diffusion_reverb_mut() {
                        e.mix = clamped;
                    }
                }
            }
        }
    }
}

fn rebuild_effect_context(
    prot_locked: &std::sync::Arc<std::sync::Mutex<crate::container::prot::Prot>>,
) -> EffectContext {
    let prot = prot_locked
        .lock()
        .unwrap_or_else(|_| panic!("prot lock poisoned — a thread panicked while holding it"));
    EffectContext {
        sample_rate: prot.info.sample_rate,
        channels: prot.info.channels as usize,
        container_path: prot.get_container_path(),
        impulse_response_spec: prot.get_impulse_response_spec(),
        impulse_response_tail_db: prot.get_impulse_response_tail_db().unwrap_or(-60.0),
    }
}
