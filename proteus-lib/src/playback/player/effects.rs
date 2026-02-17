use std::sync::atomic::Ordering;

use crate::dsp::effects::convolution_reverb::{parse_impulse_response_string, ImpulseResponseSpec};
use crate::{
    dsp::effects::AudioEffect,
    playback::engine::{DspChainMetrics, InlineEffectsUpdate},
};

use super::{Player, ReverbSettingsSnapshot};

impl Player {
    /// Override the impulse response used for convolution reverb.
    pub fn set_impulse_response_spec(&mut self, spec: ImpulseResponseSpec) {
        self.impulse_response_override = Some(spec.clone());
        let mut prot = self.prot.lock().unwrap();
        prot.set_impulse_response_spec(spec);
        self.request_effects_reset();
    }

    /// Parse and apply an impulse response spec string.
    pub fn set_impulse_response_from_string(&mut self, value: &str) {
        if let Some(spec) = parse_impulse_response_string(value) {
            self.set_impulse_response_spec(spec);
        }
    }

    /// Override the impulse response tail trim (dB).
    pub fn set_impulse_response_tail_db(&mut self, tail_db: f32) {
        self.impulse_response_tail_override = Some(tail_db);
        let mut prot = self.prot.lock().unwrap();
        prot.set_impulse_response_tail_db(tail_db);
        self.request_effects_reset();
    }

    /// Enable or disable convolution reverb.
    pub fn set_reverb_enabled(&self, enabled: bool) {
        let mut effects = self.effects.lock().unwrap();
        if let Some(effect) = effects
            .iter_mut()
            .find_map(|effect| effect.as_convolution_reverb_mut())
        {
            effect.enabled = enabled;
        }
        if let Some(effect) = effects
            .iter_mut()
            .find_map(|effect| effect.as_delay_reverb_mut())
        {
            effect.enabled = enabled;
        }
    }

    /// Set the reverb wet/dry mix (clamped to `[0.0, 1.0]`).
    pub fn set_reverb_mix(&self, dry_wet: f32) {
        let mut effects = self.effects.lock().unwrap();
        if let Some(effect) = effects
            .iter_mut()
            .find_map(|effect| effect.as_convolution_reverb_mut())
        {
            effect.dry_wet = dry_wet.clamp(0.0, 1.0);
        }
        if let Some(effect) = effects
            .iter_mut()
            .find_map(|effect| effect.as_delay_reverb_mut())
        {
            effect.mix = dry_wet.clamp(0.0, 1.0);
        }
        if let Some(effect) = effects
            .iter_mut()
            .find_map(|effect| effect.as_diffusion_reverb_mut())
        {
            effect.mix = dry_wet.clamp(0.0, 1.0);
        }
    }

    /// Retrieve the current reverb settings snapshot.
    pub fn get_reverb_settings(&self) -> ReverbSettingsSnapshot {
        let effects = self.effects.lock().unwrap();
        if let Some(effect) = effects
            .iter()
            .find_map(|effect| effect.as_convolution_reverb())
        {
            return ReverbSettingsSnapshot {
                enabled: effect.enabled,
                dry_wet: effect.dry_wet,
            };
        }
        if let Some(effect) = effects
            .iter()
            .find_map(|effect| effect.as_diffusion_reverb())
        {
            return ReverbSettingsSnapshot {
                enabled: effect.enabled,
                dry_wet: effect.mix,
            };
        }
        if let Some(effect) = effects.iter().find_map(|effect| effect.as_delay_reverb()) {
            return ReverbSettingsSnapshot {
                enabled: effect.enabled,
                dry_wet: effect.mix,
            };
        }
        ReverbSettingsSnapshot {
            enabled: false,
            dry_wet: 0.0,
        }
    }

    /// Snapshot the active effect chain names.
    #[allow(deprecated)]
    pub fn get_effect_names(&self) -> Vec<String> {
        let effects = self.effects.lock().unwrap();
        effects
            .iter()
            .map(|effect| match effect {
                AudioEffect::DelayReverb(_) => "DelayReverb".to_string(),
                AudioEffect::BasicReverb(_) => "DelayReverb".to_string(),
                AudioEffect::DiffusionReverb(_) => "DiffusionReverb".to_string(),
                AudioEffect::ConvolutionReverb(_) => "ConvolutionReverb".to_string(),
                AudioEffect::LowPassFilter(_) => "LowPassFilter".to_string(),
                AudioEffect::HighPassFilter(_) => "HighPassFilter".to_string(),
                AudioEffect::Distortion(_) => "Distortion".to_string(),
                AudioEffect::Gain(_) => "Gain".to_string(),
                AudioEffect::Compressor(_) => "Compressor".to_string(),
                AudioEffect::Limiter(_) => "Limiter".to_string(),
                AudioEffect::MultibandEq(_) => "MultibandEq".to_string(),
            })
            .collect()
    }

    /// Replace the active DSP effects chain.
    ///
    /// This method preserves legacy behavior: it forces an effect-state reset
    /// and re-seeks to the current timestamp so the new chain is applied
    /// immediately, which also refreshes the sink.
    ///
    /// # Arguments
    ///
    /// * `effects` - New ordered list of effects to apply.
    pub fn set_effects(&mut self, effects: Vec<AudioEffect>) {
        self.clear_inline_effects_update();
        self.replace_effects_chain(effects);
        self.request_effects_reset();

        // Seeking to the current timestamp refreshes the sink so that
        // the new effects are applied immediately.
        if !self.thread_finished() {
            let ts = self.get_time();
            self.seek(ts);
        }
    }

    /// Replace the active DSP effects chain inline during playback.
    ///
    /// Unlike [`Self::set_effects`], this does not reset effect internals,
    /// clear effect tails, or rebuild the sink. The updated chain settings are
    /// used for future chunks processed by the mixing thread, with a short
    /// internal crossfade to reduce boundary clicks.
    ///
    /// # Arguments
    ///
    /// * `effects` - New ordered list of effects to apply.
    pub fn set_effects_inline(&self, effects: Vec<AudioEffect>) {
        if self.thread_finished() {
            self.replace_effects_chain(effects);
            return;
        }

        let transition_ms = {
            let settings = self.buffer_settings.lock().unwrap();
            settings.inline_effects_transition_ms.max(0.0)
        };
        let mut pending = self.inline_effects_update.lock().unwrap();
        *pending = Some(InlineEffectsUpdate::new(effects, transition_ms));
    }

    /// Retrieve the latest DSP chain performance metrics.
    pub fn get_dsp_metrics(&self) -> DspChainMetrics {
        *self.dsp_metrics.lock().unwrap()
    }

    /// Retrieve the most recent per-channel peak levels.
    pub fn get_levels(&self) -> Vec<f32> {
        self.output_meter.lock().unwrap().levels()
    }

    /// Retrieve the most recent per-channel peak levels in dBFS.
    pub fn get_levels_db(&self) -> Vec<f32> {
        self.output_meter
            .lock()
            .unwrap()
            .levels()
            .into_iter()
            .map(linear_to_dbfs)
            .collect()
    }

    /// Retrieve the most recent per-channel average levels.
    pub fn get_levels_avg(&self) -> Vec<f32> {
        self.output_meter.lock().unwrap().averages()
    }

    /// Set the output meter refresh rate (frames per second).
    pub fn set_output_meter_refresh_hz(&self, hz: f32) {
        self.output_meter.lock().unwrap().set_refresh_hz(hz);
    }

    pub(super) fn request_effects_reset(&self) {
        self.effects_reset.fetch_add(1, Ordering::SeqCst);
    }

    pub(super) fn clear_inline_effects_update(&self) {
        let mut pending = self.inline_effects_update.lock().unwrap();
        pending.take();
    }

    fn replace_effects_chain(&self, effects: Vec<AudioEffect>) {
        let mut guard = self.effects.lock().unwrap();
        log::info!("updated effects chain: {} effect(s)", effects.len());
        *guard = effects;
    }
}

fn linear_to_dbfs(value: f32) -> f32 {
    if value <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * value.log10()
    }
}
