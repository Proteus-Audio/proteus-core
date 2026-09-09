//! DSP-chain control and output metering accessors for `Player`.
//!
//! Methods in this module mutate effect configuration, trigger effect resets,
//! and expose meter/metrics snapshots suitable for UI polling.

use std::sync::atomic::Ordering;

use crate::dsp::effects::convolution_reverb::{parse_impulse_response_string, ImpulseResponseSpec};
use crate::{
    dsp::effects::{normalize_legacy_effect_aliases, AudioEffect},
    playback::engine::{
        DspChainMetrics, EffectParameter, EffectSettingsCommand, InlineEffectsUpdate,
    },
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
        let mut prot = self.lock_prot_invariant();
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
        let mut prot = self.lock_prot_invariant();
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
        let mut effects = self.lock_effects_recoverable();
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
        let mut effects = self.lock_effects_recoverable();
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
        let effects = self.lock_effects_recoverable();
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
        let effects = self.lock_effects_recoverable();
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
            let settings = self.lock_buffer_settings_recoverable();
            settings.inline_effects_transition_ms.max(0.0)
        };
        let mut pending = self.lock_inline_effects_update_recoverable();
        *pending = Some(InlineEffectsUpdate::new(effects, transition_ms));
    }

    /// Retrieve the latest DSP chain performance metrics.
    ///
    /// # Returns
    ///
    /// A copy of the most recent metrics updated by the playback thread.
    pub fn get_dsp_metrics(&self) -> DspChainMetrics {
        *self.lock_dsp_metrics_recoverable()
    }

    /// Retrieve the most recent per-channel peak levels.
    pub fn get_levels(&self) -> Vec<f32> {
        self.lock_output_meter_recoverable().levels()
    }

    /// Retrieve the most recent per-channel peak levels in dBFS.
    pub fn get_levels_db(&self) -> Vec<f32> {
        self.lock_output_meter_recoverable()
            .levels()
            .into_iter()
            .map(linear_to_dbfs)
            .collect()
    }

    /// Retrieve the most recent per-channel average levels.
    pub fn get_levels_avg(&self) -> Vec<f32> {
        self.lock_output_meter_recoverable().averages()
    }

    /// Set the output meter refresh rate (frames per second).
    pub fn set_output_meter_refresh_hz(&self, hz: f32) {
        self.lock_output_meter_recoverable().set_refresh_hz(hz);
    }

    /// Bump the effects reset generation consumed by the runtime engine.
    pub(super) fn request_effects_reset(&self) {
        self.effects_reset.fetch_add(1, Ordering::SeqCst);
    }

    /// Enqueue an incremental effect settings command for the mix thread.
    fn push_effect_settings_command(&self, command: EffectSettingsCommand) {
        self.lock_effect_settings_commands_recoverable()
            .push(command);
    }

    /// Drop any pending inline effects transition update.
    pub(super) fn clear_inline_effects_update(&self) {
        let mut pending = self.lock_inline_effects_update_recoverable();
        pending.take();
    }

    /// Update a single parameter on the effect at `index` in the chain.
    ///
    /// The update is queued for the mix thread and also applied to the shared
    /// chain so that control-path reads reflect the new value immediately.
    ///
    /// # Arguments
    ///
    /// * `index` - Zero-based index into the effect chain.
    /// * `param` - The specific parameter value to update.
    ///
    /// # Returns
    ///
    /// `false` if `index` is out of range, `true` otherwise.
    pub fn set_effect_parameter(&self, index: usize, param: EffectParameter) -> bool {
        let effects = self.lock_effects_recoverable();
        if index >= effects.len() {
            return false;
        }
        drop(effects);
        self.push_effect_settings_command(EffectSettingsCommand::SetEffectParameter {
            effect_index: index,
            parameter: param.clone(),
        });
        // Mirror the update on the shared chain for UI reads.
        let mut effects = self.lock_effects_recoverable();
        apply_effect_parameter_shared(&mut effects[index], param);
        true
    }

    /// Toggle enabled/disabled for the effect at `index` in the chain.
    ///
    /// The update is queued for the mix thread and also applied to the shared
    /// chain so that control-path reads reflect the new value immediately.
    ///
    /// # Arguments
    ///
    /// * `index` - Zero-based index into the effect chain.
    /// * `enabled` - New enabled state.
    ///
    /// # Returns
    ///
    /// `false` if `index` is out of range, `true` otherwise.
    pub fn set_effect_enabled(&self, index: usize, enabled: bool) -> bool {
        let effects = self.lock_effects_recoverable();
        if index >= effects.len() {
            return false;
        }
        drop(effects);
        self.push_effect_settings_command(EffectSettingsCommand::SetEffectEnabled {
            effect_index: index,
            enabled,
        });
        let mut effects = self.lock_effects_recoverable();
        set_effect_enabled_shared(&mut effects[index], enabled);
        true
    }

    /// Replace the currently active effect vector atomically.
    fn replace_effects_chain(&self, effects: Vec<AudioEffect>) {
        let mut guard = self.lock_effects_recoverable();
        let normalized = normalize_legacy_effect_aliases(effects);
        log::info!("updated effects chain: {} effect(s)", normalized.len());
        let effect_count = normalized.len();
        *guard = normalized;
        self.effect_meter
            .set_level_layout_zeroed(effect_count, self.info.channels as usize);
        self.effect_meter.set_spectral_layout_zeroed(effect_count);
    }
}

fn apply_effect_parameter_shared(effect: &mut AudioEffect, param: EffectParameter) {
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

fn set_effect_enabled_shared(effect: &mut AudioEffect, enabled: bool) {
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
    use crate::container::prot::PathsTrack;
    use crate::dsp::effects::{AudioEffect, DelayReverbEffect, GainEffect, PanEffect};
    use crate::playback::engine::{EffectParameter, EffectSettingsCommand};
    use crate::playback::player::{Player, PlayerState};
    use std::sync::atomic::Ordering;

    #[test]
    fn zero_or_negative_linear_maps_to_negative_infinity() {
        assert!(linear_to_dbfs(0.0).is_infinite() && linear_to_dbfs(0.0).is_sign_negative());
        assert!(linear_to_dbfs(-1.0).is_infinite() && linear_to_dbfs(-1.0).is_sign_negative());
    }

    #[test]
    fn unity_linear_maps_to_zero_dbfs() {
        assert_eq!(linear_to_dbfs(1.0), 0.0);
    }

    #[test]
    fn set_effect_parameter_updates_gain_pan_and_reverb_mix() {
        let player = test_player(vec![
            AudioEffect::Gain(GainEffect::default()),
            AudioEffect::Pan(PanEffect::default()),
            AudioEffect::DelayReverb(DelayReverbEffect::default()),
        ]);

        assert!(player.set_effect_parameter(0, EffectParameter::Gain(1.5)));
        assert!(player.set_effect_parameter(1, EffectParameter::Pan(0.75)));
        assert!(player.set_effect_parameter(2, EffectParameter::ReverbMix(0.6)));

        let effects = player.lock_effects_recoverable();
        match &effects[0] {
            AudioEffect::Gain(effect) => assert!((effect.settings.gain - 1.5).abs() < 1e-6),
            _ => panic!("expected gain effect"),
        }
        match &effects[1] {
            AudioEffect::Pan(effect) => assert!((effect.settings.pan - 0.75).abs() < 1e-6),
            _ => panic!("expected pan effect"),
        }
        match &effects[2] {
            AudioEffect::DelayReverb(effect) => assert!((effect.mix - 0.6).abs() < 1e-6),
            _ => panic!("expected delay reverb effect"),
        }
        drop(effects);

        let commands = player.lock_effect_settings_commands_recoverable();
        assert!(matches!(
            commands[0],
            EffectSettingsCommand::SetEffectParameter {
                effect_index: 0,
                parameter: EffectParameter::Gain(_)
            }
        ));
        assert!(matches!(
            commands[1],
            EffectSettingsCommand::SetEffectParameter {
                effect_index: 1,
                parameter: EffectParameter::Pan(_)
            }
        ));
        assert!(matches!(
            commands[2],
            EffectSettingsCommand::SetEffectParameter {
                effect_index: 2,
                parameter: EffectParameter::ReverbMix(_)
            }
        ));
    }

    #[test]
    fn set_effect_parameter_returns_false_for_out_of_range_index() {
        let player = test_player(vec![AudioEffect::Gain(GainEffect::default())]);
        assert!(!player.set_effect_parameter(3, EffectParameter::Gain(2.0)));
    }

    fn test_player(effects: Vec<AudioEffect>) -> Player {
        let player = Player::new_from_file_paths(vec![PathsTrack::new_from_file_paths(vec![
            "/tmp/nonexistent.wav".to_string(),
        ])]);
        player.playback_thread_exists.store(false, Ordering::SeqCst);
        player.abort.store(true, Ordering::SeqCst);
        *player.lock_playback_thread_handle_invariant() = None;
        *player.lock_state_invariant() = PlayerState::Stopped;
        *player.lock_effects_recoverable() = effects;
        player.lock_effect_settings_commands_recoverable().clear();
        player
    }
}
