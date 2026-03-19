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

use super::super::effects::{audio_effect_enabled, run_effect_chain, EffectEnableFade};
use super::super::output_stage;
use super::super::types::{EffectParameter, EffectSettingsCommand};
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
    process_effects(samples.as_slice(), state);
    #[cfg(feature = "debug")]
    update_debug_metrics(
        state,
        dsp_start,
        audio_time_ms,
        state.effect_scratch_a.len(),
    );
    let slice_samples = output_slice_samples(state);
    match output_stage::send_samples(
        &state.sender,
        state.audio_info.channels as u16,
        state.audio_info.sample_rate,
        &state.effect_scratch_a,
        slice_samples,
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
    let mut metrics = state.lock_dsp_metrics_recoverable();
    metrics.track_key_count = state.buffer_mixer.instance_count();
    metrics.prot_key_count = state.buffer_mixer.logical_track_count();
    metrics.finished_track_count = state.buffer_mixer.finished_instance_count();
    true
}

fn process_effects(samples: &[f32], state: &mut MixLoopState) {
    if let Some(transition) = state.active_inline_transition.as_mut() {
        // Run old effects chain; result ends up in scratch_a.
        run_effect_chain(
            &mut transition.old_effects,
            samples,
            &state.effect_context,
            false,
            &mut state.effect_scratch_a,
            &mut state.effect_scratch_b,
            None,
            None,
        );
        // During a transition we need both outputs simultaneously, so we save
        // old_out in a temporary Vec. Transitions are non-steady-state so this
        // single allocation per chunk is acceptable.
        let old_out: Vec<f32> = state.effect_scratch_a.clone();

        // Run new effects chain; result ends up in scratch_a.
        run_effect_chain(
            &mut transition.new_effects,
            samples,
            &state.effect_context,
            false,
            &mut state.effect_scratch_a,
            &mut state.effect_scratch_b,
            None,
            None,
        );

        let len = old_out.len().max(state.effect_scratch_a.len());
        state.effect_scratch_b.clear();
        state.effect_scratch_b.reserve(len);
        let mix = if transition.total_samples == 0 {
            1.0
        } else {
            let done = transition
                .total_samples
                .saturating_sub(transition.remaining_samples);
            (done as f32 / transition.total_samples as f32).clamp(0.0, 1.0)
        };
        for i in 0..len {
            let o = old_out.get(i).copied().unwrap_or(0.0);
            let n = state.effect_scratch_a.get(i).copied().unwrap_or(0.0);
            state.effect_scratch_b.push((o * (1.0 - mix)) + (n * mix));
        }
        std::mem::swap(&mut state.effect_scratch_a, &mut state.effect_scratch_b);

        transition.remaining_samples = transition
            .remaining_samples
            .saturating_sub(samples.len().max(1));
    } else {
        let frames = samples.len() / state.effect_context.channels().max(1);
        let mut metering = state.effect_metering.prepare_chunk(
            &state.local_effects,
            &state.effect_context,
            frames,
        );
        let observer = metering
            .as_mut()
            .map(|metering| metering as &mut dyn super::super::effects::EffectChainObserver);
        // DSP runs on the mix-thread-owned local chain — no mutex held.
        run_effect_chain(
            &mut state.local_effects,
            samples,
            &state.effect_context,
            false,
            &mut state.effect_scratch_a,
            &mut state.effect_scratch_b,
            observer,
            Some(&mut state.effect_enable_fades),
        );
        if let Some(metering) = metering {
            metering.finish(&state.local_effects);
        }
    }

    // Finalize transition: adopt new effects as the local chain and sync shared.
    if state
        .active_inline_transition
        .as_ref()
        .is_some_and(|transition| transition.remaining_samples == 0)
    {
        if let Some(transition) = state.active_inline_transition.take() {
            let completed = transition.new_effects;
            *state.lock_effects_recoverable() = completed.clone();
            state.local_effects = completed;
            state.effect_enable_fades = vec![None; state.local_effects.len()];
            state.effect_metering.reset_for_chain(
                &state.local_effects,
                &state.effect_context,
                true,
            );
        }
    }
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
    let mut metrics = state.lock_dsp_metrics_recoverable();
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

    drain_effect_chains(state);

    if state.effect_scratch_a.is_empty() {
        return false;
    }

    let max_abs = state
        .effect_scratch_a
        .iter()
        .fold(0.0_f32, |acc, s| acc.max(s.abs()));
    if max_abs <= DRAIN_SILENCE_EPSILON {
        state.effect_drain_silent_passes = state.effect_drain_silent_passes.saturating_add(1);
    } else {
        state.effect_drain_silent_passes = 0;
    }
    if state.effect_drain_silent_passes >= DRAIN_SILENT_PASSES_TO_STOP {
        info!("effect drain stopped after consecutive silent drain passes");
        return false;
    }

    let slice_samples = output_slice_samples(state);
    match output_stage::send_samples(
        &state.sender,
        state.audio_info.channels as u16,
        state.audio_info.sample_rate,
        &state.effect_scratch_a,
        slice_samples,
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
    sync_effect_context_from_buffer_settings(state);

    // Drain incremental settings commands from the control path.
    drain_effect_settings_commands(state);

    let current_reset = state.effects_reset.load(Ordering::SeqCst);
    if current_reset != state.last_effects_reset {
        // Full reset: clone the new chain from shared into local.
        let refreshed_effects = state.lock_effects_recoverable().clone();
        state.local_effects = refreshed_effects;
        for effect in state.local_effects.iter_mut() {
            effect.reset_state();
        }
        state.effect_enable_fades = vec![None; state.local_effects.len()];
        state.active_inline_transition = None;
        state.lock_inline_effects_update_recoverable().take();
        state.effect_context = rebuild_effect_context(&state.prot, &state.buffer_settings);
        state.last_effects_reset = current_reset;
        state
            .effect_metering
            .reset_for_chain(&state.local_effects, &state.effect_context, true);
    }

    let pending_update = {
        let mut pending = state.lock_inline_effects_update_recoverable();
        pending.take()
    };
    if let Some(update) = pending_update {
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
            *state.lock_effects_recoverable() = state.local_effects.clone();
            state.effect_enable_fades = vec![None; state.local_effects.len()];
            state.active_inline_transition = None;
            state.effect_metering.reset_for_chain(
                &state.local_effects,
                &state.effect_context,
                true,
            );
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

fn drain_effect_chains(state: &mut MixLoopState) {
    if let Some(transition) = state.active_inline_transition.as_mut() {
        run_effect_chain(
            &mut transition.old_effects,
            &[],
            &state.effect_context,
            true,
            &mut state.effect_scratch_a,
            &mut state.effect_scratch_b,
            None,
            None,
        );
        let old_out: Vec<f32> = state.effect_scratch_a.clone();

        run_effect_chain(
            &mut transition.new_effects,
            &[],
            &state.effect_context,
            true,
            &mut state.effect_scratch_a,
            &mut state.effect_scratch_b,
            None,
            None,
        );

        let len = old_out.len().max(state.effect_scratch_a.len());
        state.effect_scratch_b.clear();
        for i in 0..len {
            state.effect_scratch_b.push(
                (old_out.get(i).copied().unwrap_or(0.0)
                    + state.effect_scratch_a.get(i).copied().unwrap_or(0.0))
                    * 0.5,
            );
        }
        std::mem::swap(&mut state.effect_scratch_a, &mut state.effect_scratch_b);
    } else {
        // Drain runs on the local chain — no mutex held.
        run_effect_chain(
            &mut state.local_effects,
            &[],
            &state.effect_context,
            true,
            &mut state.effect_scratch_a,
            &mut state.effect_scratch_b,
            None,
            Some(&mut state.effect_enable_fades),
        );
    }
}

/// Drain queued effect settings commands and apply them to the local chain.
fn drain_effect_settings_commands(state: &mut MixLoopState) {
    let commands = {
        let mut pending = state.lock_effect_settings_commands_recoverable();
        if pending.is_empty() {
            return;
        }
        std::mem::take(&mut *pending)
    };
    for command in commands {
        match command {
            EffectSettingsCommand::SetReverbEnabled(enabled) => {
                let mut indices = Vec::new();
                for (index, effect) in state.local_effects.iter().enumerate() {
                    if effect.as_convolution_reverb().is_some()
                        || effect.as_delay_reverb().is_some()
                        || effect.as_diffusion_reverb().is_some()
                    {
                        indices.push(index);
                    }
                }
                for index in indices {
                    schedule_effect_enable_fade(state, index, enabled);
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
            EffectSettingsCommand::SetEffectParameter {
                effect_index,
                parameter,
            } => {
                if let Some(effect) = state.local_effects.get_mut(effect_index) {
                    apply_effect_parameter(effect, parameter);
                }
            }
            EffectSettingsCommand::SetEffectEnabled {
                effect_index,
                enabled,
            } => {
                schedule_effect_enable_fade(state, effect_index, enabled);
            }
        }
    }
}

fn schedule_effect_enable_fade(state: &mut MixLoopState, effect_index: usize, enabled: bool) {
    let Some(effect) = state.local_effects.get_mut(effect_index) else {
        return;
    };

    let current_mix = state
        .effect_enable_fades
        .get(effect_index)
        .and_then(Option::as_ref)
        .map_or_else(
            || {
                if audio_effect_enabled(effect) {
                    1.0
                } else {
                    0.0
                }
            },
            EffectEnableFade::current_mix,
        );
    let target_mix = if enabled { 1.0 } else { 0.0 };
    if (current_mix - target_mix).abs() < f32::EPSILON {
        if !enabled {
            effect.reset_state();
        }
        set_effect_enabled(effect, enabled);
        if let Some(slot) = state.effect_enable_fades.get_mut(effect_index) {
            *slot = None;
        }
        return;
    }

    if enabled && !audio_effect_enabled(effect) && current_mix <= f32::EPSILON {
        effect.reset_state();
        set_effect_enabled(effect, true);
    }

    let ramp_frames = state.effect_context.parameter_ramp_samples();
    if ramp_frames == 0 {
        if !enabled {
            effect.reset_state();
        }
        set_effect_enabled(effect, enabled);
        state.effect_enable_fades[effect_index] = None;
        return;
    }

    state.effect_enable_fades[effect_index] =
        Some(EffectEnableFade::new(current_mix, enabled, ramp_frames));
}

fn apply_effect_parameter(effect: &mut crate::dsp::effects::AudioEffect, param: EffectParameter) {
    use crate::dsp::effects::AudioEffect;
    match param {
        EffectParameter::Gain(v) => {
            if let AudioEffect::Gain(e) = effect {
                e.settings.gain = v;
            }
        }
        EffectParameter::Pan(v) => {
            if let AudioEffect::Pan(e) = effect {
                e.settings.pan = v;
            }
        }
        EffectParameter::ReverbMix(v) => {
            let clamped = v.clamp(0.0, 1.0);
            match effect {
                AudioEffect::ConvolutionReverb(e) => e.dry_wet = clamped,
                AudioEffect::DelayReverb(e) => e.mix = clamped,
                AudioEffect::DiffusionReverb(e) => e.mix = clamped,
                _ => {}
            }
        }
        EffectParameter::DistortionGain(v) => {
            if let AudioEffect::Distortion(e) = effect {
                e.settings.gain = v;
            }
        }
        EffectParameter::DistortionThreshold(v) => {
            if let AudioEffect::Distortion(e) = effect {
                e.settings.threshold = v;
            }
        }
        EffectParameter::LowPassFreqHz(v) => {
            if let AudioEffect::LowPassFilter(e) = effect {
                e.settings.freq_hz = v;
            }
        }
        EffectParameter::LowPassQ(v) => {
            if let AudioEffect::LowPassFilter(e) = effect {
                e.settings.q = v;
            }
        }
        EffectParameter::HighPassFreqHz(v) => {
            if let AudioEffect::HighPassFilter(e) = effect {
                e.settings.freq_hz = v;
            }
        }
        EffectParameter::HighPassQ(v) => {
            if let AudioEffect::HighPassFilter(e) = effect {
                e.settings.q = v;
            }
        }
        EffectParameter::CompressorThresholdDb(v) => {
            if let AudioEffect::Compressor(e) = effect {
                e.settings.threshold_db = v;
            }
        }
        EffectParameter::CompressorRatio(v) => {
            if let AudioEffect::Compressor(e) = effect {
                e.settings.ratio = v;
            }
        }
        EffectParameter::CompressorAttackMs(v) => {
            if let AudioEffect::Compressor(e) = effect {
                e.settings.attack_ms = v;
            }
        }
        EffectParameter::CompressorReleaseMs(v) => {
            if let AudioEffect::Compressor(e) = effect {
                e.settings.release_ms = v;
            }
        }
        EffectParameter::CompressorMakeupDb(v) => {
            if let AudioEffect::Compressor(e) = effect {
                e.settings.makeup_gain_db = v;
            }
        }
        EffectParameter::LimiterThresholdDb(v) => {
            if let AudioEffect::Limiter(e) = effect {
                e.settings.threshold_db = v;
            }
        }
        EffectParameter::LimiterKneeWidthDb(v) => {
            if let AudioEffect::Limiter(e) = effect {
                e.settings.knee_width_db = v;
            }
        }
        EffectParameter::LimiterAttackMs(v) => {
            if let AudioEffect::Limiter(e) = effect {
                e.settings.attack_ms = v;
            }
        }
        EffectParameter::LimiterReleaseMs(v) => {
            if let AudioEffect::Limiter(e) = effect {
                e.settings.release_ms = v;
            }
        }
    }
}

fn set_effect_enabled(effect: &mut crate::dsp::effects::AudioEffect, enabled: bool) {
    use crate::dsp::effects::AudioEffect;
    match effect {
        AudioEffect::Gain(e) => e.enabled = enabled,
        AudioEffect::Pan(e) => e.enabled = enabled,
        AudioEffect::Distortion(e) => e.enabled = enabled,
        AudioEffect::DelayReverb(e) => e.enabled = enabled,
        AudioEffect::DiffusionReverb(e) => e.enabled = enabled,
        AudioEffect::ConvolutionReverb(e) => e.enabled = enabled,
        AudioEffect::LowPassFilter(e) => e.enabled = enabled,
        AudioEffect::HighPassFilter(e) => e.enabled = enabled,
        AudioEffect::Compressor(e) => e.enabled = enabled,
        AudioEffect::Limiter(e) => e.enabled = enabled,
        AudioEffect::MultibandEq(e) => e.enabled = enabled,
    }
}

fn rebuild_effect_context(
    prot_locked: &std::sync::Arc<std::sync::Mutex<crate::container::prot::Prot>>,
    buffer_settings: &std::sync::Arc<
        std::sync::Mutex<crate::playback::engine::PlaybackBufferSettings>,
    >,
) -> EffectContext {
    let prot = crate::playback::mutex_policy::lock_invariant(
        prot_locked,
        "mix runtime prot",
        "effect context rebuilds require coherent container metadata",
    );
    let parameter_ramp_ms = crate::playback::mutex_policy::lock_recoverable(
        buffer_settings,
        "mix runtime buffer settings",
        "buffer settings are runtime configuration snapshots",
    )
    .parameter_ramp_ms;
    let mut context = EffectContext::new(
        prot.info.sample_rate,
        prot.info.channels as usize,
        prot.get_container_path(),
        prot.get_impulse_response_spec(),
        prot.get_impulse_response_tail_db().unwrap_or(-60.0),
    )
    .expect("prot info must have valid sample rate and channel count");
    context.set_parameter_ramp_ms(parameter_ramp_ms);
    context
}

fn output_slice_samples(state: &MixLoopState) -> Option<usize> {
    state
        .lock_buffer_settings_recoverable()
        .output_slice_ms
        .map(|ms| {
            let channels = state.audio_info.channels.max(1) as usize;
            let frames = (state.audio_info.sample_rate as f32 * ms / 1000.0).ceil() as usize;
            (frames * channels).max(channels)
        })
}

fn sync_effect_context_from_buffer_settings(state: &mut MixLoopState) {
    let parameter_ramp_ms = state.lock_buffer_settings_recoverable().parameter_ramp_ms;
    state
        .effect_context
        .set_parameter_ramp_ms(parameter_ramp_ms);
}
