//! DSP-chain control and output metering accessors for `Player`.
//!
//! Methods in this module mutate effect configuration, trigger effect resets,
//! and expose meter/metrics snapshots suitable for UI polling.

use std::sync::atomic::Ordering;

use crate::dsp::effects::convolution_reverb::{parse_impulse_response_string, ImpulseResponseSpec};
use crate::{
    dsp::effects::{normalize_legacy_effect_aliases, AudioEffect},
    playback::engine::{DspChainMetrics, EffectSettingsCommand, InlineEffectsUpdate},
};

use super::{Player, ReverbSettingsSnapshot};

impl Player {
    /// Override the impulse response used for convolution reverb.
    ///
    /// # Arguments
    ///
    /// * `spec` - Parsed IR selection/configuration to persist on the player.
    pub fn set_impulse_response_spec(&mut self, spec: ImpulseResponseSpec) {
        self.impulse_response_override = Some(spec.clone());
        let mut prot = self
            .prot
            .lock()
            .unwrap_or_else(|_| panic!("prot lock poisoned — a thread panicked while holding it"));
        prot.set_impulse_response_spec(spec);
        self.request_effects_reset();
    }

    /// Parse and apply an impulse response spec string.
    ///
    /// Invalid input is ignored and leaves the current override unchanged.
    pub fn set_impulse_response_from_string(&mut self, value: &str) {
        if let Some(spec) = parse_impulse_response_string(value) {
            self.set_impulse_response_spec(spec);
        }
    }

    /// Override the impulse response tail trim (dB).
    ///
    /// # Arguments
    ///
    /// * `tail_db` - Trim threshold in decibels applied to IR tails.
    pub fn set_impulse_response_tail_db(&mut self, tail_db: f32) {
        self.impulse_response_tail_override = Some(tail_db);
        let mut prot = self
            .prot
            .lock()
            .unwrap_or_else(|_| panic!("prot lock poisoned — a thread panicked while holding it"));
        prot.set_impulse_response_tail_db(tail_db);
        self.request_effects_reset();
    }

    /// Enable or disable supported reverb effects in the active chain.
    ///
    /// The toggle is applied to convolution and delay-reverb instances when
    /// present. The update is queued for the mix thread and also applied to the
    /// shared chain so that control-path reads reflect the new value.
    pub fn set_reverb_enabled(&self, enabled: bool) {
        self.push_effect_settings_command(EffectSettingsCommand::SetReverbEnabled(enabled));
        let mut effects = self.effects.lock().unwrap_or_else(|_| {
            panic!("effects lock poisoned — a thread panicked while holding it")
        });
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
    ///
    /// The value is mapped across convolution, delay, and diffusion reverb
    /// variants when those effects are part of the chain. The update is queued
    /// for the mix thread and also applied to the shared chain for reads.
    pub fn set_reverb_mix(&self, dry_wet: f32) {
        self.push_effect_settings_command(EffectSettingsCommand::SetReverbMix(dry_wet));
        let mut effects = self.effects.lock().unwrap_or_else(|_| {
            panic!("effects lock poisoned — a thread panicked while holding it")
        });
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
    ///
    /// Returns a disabled/zeroed snapshot when no known reverb effect exists.
    pub fn get_reverb_settings(&self) -> ReverbSettingsSnapshot {
        let effects = self.effects.lock().unwrap_or_else(|_| {
            panic!("effects lock poisoned — a thread panicked while holding it")
        });
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
    ///
    /// This is primarily intended for diagnostics and UI display.
    pub fn get_effect_names(&self) -> Vec<String> {
        let effects = self.effects.lock().unwrap_or_else(|_| {
            panic!("effects lock poisoned — a thread panicked while holding it")
        });
        effects
            .iter()
            .map(|effect| effect.display_name().to_string())
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
            let settings = self.buffer_settings.lock().unwrap_or_else(|_| {
                panic!("buffer settings lock poisoned — a thread panicked while holding it")
            });
            settings.inline_effects_transition_ms.max(0.0)
        };
        let mut pending = self.inline_effects_update.lock().unwrap_or_else(|_| {
            panic!("inline effects update lock poisoned — a thread panicked while holding it")
        });
        *pending = Some(InlineEffectsUpdate::new(effects, transition_ms));
    }

    /// Retrieve the latest DSP chain performance metrics.
    ///
    /// # Returns
    ///
    /// A copy of the most recent metrics updated by the playback thread.
    pub fn get_dsp_metrics(&self) -> DspChainMetrics {
        *self.dsp_metrics.lock().unwrap_or_else(|_| {
            panic!("dsp metrics lock poisoned — a thread panicked while holding it")
        })
    }

    /// Retrieve the most recent per-channel peak levels.
    pub fn get_levels(&self) -> Vec<f32> {
        self.output_meter
            .lock()
            .unwrap_or_else(|_| {
                panic!("output meter lock poisoned — a thread panicked while holding it")
            })
            .levels()
    }

    /// Retrieve the most recent per-channel peak levels in dBFS.
    pub fn get_levels_db(&self) -> Vec<f32> {
        self.output_meter
            .lock()
            .unwrap_or_else(|_| {
                panic!("output meter lock poisoned — a thread panicked while holding it")
            })
            .levels()
            .into_iter()
            .map(linear_to_dbfs)
            .collect()
    }

    /// Retrieve the most recent per-channel average levels.
    pub fn get_levels_avg(&self) -> Vec<f32> {
        self.output_meter
            .lock()
            .unwrap_or_else(|_| {
                panic!("output meter lock poisoned — a thread panicked while holding it")
            })
            .averages()
    }

    /// Set the output meter refresh rate (frames per second).
    pub fn set_output_meter_refresh_hz(&self, hz: f32) {
        self.output_meter
            .lock()
            .unwrap_or_else(|_| {
                panic!("output meter lock poisoned — a thread panicked while holding it")
            })
            .set_refresh_hz(hz);
    }

    /// Bump the effects reset generation consumed by the runtime engine.
    pub(super) fn request_effects_reset(&self) {
        self.effects_reset.fetch_add(1, Ordering::SeqCst);
    }

    /// Enqueue an incremental effect settings command for the mix thread.
    fn push_effect_settings_command(&self, command: EffectSettingsCommand) {
        self.effect_settings_commands
            .lock()
            .unwrap_or_else(|_| {
                panic!(
                    "effect settings commands lock poisoned — a thread panicked while holding it"
                )
            })
            .push(command);
    }

    /// Drop any pending inline effects transition update.
    pub(super) fn clear_inline_effects_update(&self) {
        let mut pending = self.inline_effects_update.lock().unwrap_or_else(|_| {
            panic!("inline effects update lock poisoned — a thread panicked while holding it")
        });
        pending.take();
    }

    /// Replace the currently active effect vector atomically.
    fn replace_effects_chain(&self, effects: Vec<AudioEffect>) {
        let mut guard = self.effects.lock().unwrap_or_else(|_| {
            panic!("effects lock poisoned — a thread panicked while holding it")
        });
        let normalized = normalize_legacy_effect_aliases(effects);
        log::info!("updated effects chain: {} effect(s)", normalized.len());
        *guard = normalized;
    }
}

fn linear_to_dbfs(value: f32) -> f32 {
    if value <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * value.log10()
    }
}

#[cfg(test)]
mod tests {
    use super::linear_to_dbfs;

    #[test]
    fn zero_or_negative_linear_maps_to_negative_infinity() {
        assert!(linear_to_dbfs(0.0).is_infinite() && linear_to_dbfs(0.0).is_sign_negative());
        assert!(linear_to_dbfs(-1.0).is_infinite() && linear_to_dbfs(-1.0).is_sign_negative());
    }

    #[test]
    fn unity_linear_maps_to_zero_dbfs() {
        assert_eq!(linear_to_dbfs(1.0), 0.0);
    }
}
