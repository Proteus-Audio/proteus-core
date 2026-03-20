//! Per-effect metering and analytical response accessors for `Player`.

use crate::dsp::meter::{EffectBandSnapshot, EffectLevelSnapshot, FilterResponseCurve};

use super::Player;

impl Player {
    /// Enable or disable runtime per-effect input/output level metering.
    ///
    /// Disabled builds ignore this setter.
    pub fn set_effect_level_metering_enabled(&self, enabled: bool) {
        self.effect_meter.set_level_metering_enabled(enabled);
        if enabled {
            let effect_count = self.lock_effects_recoverable().len();
            self.effect_meter
                .set_level_layout_zeroed(effect_count, self.info.channels as usize);
        }
    }

    /// Set the runtime per-effect level-meter refresh cadence in Hz.
    ///
    /// Disabled builds ignore this setter.
    pub fn set_effect_level_meter_refresh_hz(&self, hz: f32) {
        self.effect_meter.set_level_refresh_hz(hz);
    }

    /// Return the latest per-effect input/output level snapshots.
    ///
    /// Returns the most recently processed mix-thread snapshot regardless of
    /// output-path latency. Suitable for offline/diagnostic use.
    ///
    /// Returns `None` when the `effect-meter` feature is not compiled in or
    /// when runtime level metering is disabled.
    pub fn effect_levels(&self) -> Option<Vec<EffectLevelSnapshot>> {
        self.effect_meter.effect_levels()
    }

    /// Return per-effect level snapshots aligned to audible playback time.
    ///
    /// During live playback the mix thread runs ahead of the listener.
    /// This accessor returns the snapshot whose audio is closest to the
    /// chunk currently draining from the managed sink queue, rather than
    /// the latest chunk the mix thread has processed.
    ///
    /// Returns `None` when the `effect-meter` feature is not compiled in,
    /// when runtime level metering is disabled, or when no snapshot has
    /// been produced yet.
    pub fn effect_levels_audible(&self) -> Option<Vec<EffectLevelSnapshot>> {
        let audible_time = self.get_time();
        self.effect_meter.effect_levels_audible(audible_time)
    }

    /// Build analytical filter-response curves from the current effect settings.
    ///
    /// Non-filter effects return `None` in their slot. Builds without the
    /// `effect-meter` feature return `None` for every slot.
    pub fn effect_frequency_responses(
        &self,
        num_points: usize,
    ) -> Vec<Option<FilterResponseCurve>> {
        let effects = self.lock_effects_recoverable();
        effects
            .iter()
            .map(|effect| effect.frequency_response_curve(self.info.sample_rate, num_points))
            .collect()
    }

    /// Enable or disable runtime FFT-based spectral analysis for filter effects.
    ///
    /// Disabled builds ignore this setter.
    pub fn set_spectral_analysis_enabled(&self, enabled: bool) {
        if self.effect_meter.spectral_analysis_enabled() == enabled {
            return;
        }
        self.effect_meter.set_spectral_analysis_enabled(enabled);
        if enabled {
            let effect_count = self.lock_effects_recoverable().len();
            self.effect_meter.set_spectral_layout_zeroed(effect_count);
        }
    }

    /// Set the runtime spectral-analysis refresh cadence in Hz.
    ///
    /// Disabled builds ignore this setter.
    pub fn set_spectral_analysis_refresh_hz(&self, hz: f32) {
        self.effect_meter.set_spectral_refresh_hz(hz);
    }

    /// Return whether runtime spectral analysis is currently enabled.
    pub fn spectral_analysis_enabled(&self) -> bool {
        self.effect_meter.spectral_analysis_enabled()
    }

    /// Return the latest spectral-analysis snapshots for filter effects.
    ///
    /// Buckets for multiband EQ are control-aligned analysis buckets derived
    /// from the visible section frequencies; they are not exact isolated
    /// per-filter contributions.
    ///
    /// Returns `None` when `effect-meter-spectral` is not compiled in or when
    /// runtime spectral analysis is disabled.
    pub fn effect_band_levels(&self) -> Option<Vec<Option<EffectBandSnapshot>>> {
        self.effect_meter.effect_band_levels()
    }

    /// Return spectral snapshots aligned to audible playback time.
    ///
    /// During live playback the mix thread runs ahead of the listener.
    /// This accessor returns the spectral snapshot whose audio is closest
    /// to the chunk currently draining from the managed sink queue.
    ///
    /// Returns `None` when `effect-meter-spectral` is not compiled in, when
    /// runtime spectral analysis is disabled, or when no snapshot has been
    /// produced yet.
    pub fn effect_band_levels_audible(&self) -> Option<Vec<Option<EffectBandSnapshot>>> {
        let audible_time = self.get_time();
        self.effect_meter.effect_band_levels_audible(audible_time)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use crate::container::prot::PathsTrack;
    use crate::dsp::effects::{AudioEffect, GainEffect, LowPassFilterEffect};
    use crate::playback::player::{Player, PlayerState};

    #[test]
    fn effect_levels_return_none_while_runtime_metering_is_disabled() {
        let player = test_player(vec![AudioEffect::Gain(GainEffect::default())]);
        assert_eq!(player.effect_levels(), None);
    }

    #[test]
    fn effect_levels_audible_returns_none_while_metering_disabled() {
        let player = test_player(vec![AudioEffect::Gain(GainEffect::default())]);
        assert_eq!(player.effect_levels_audible(), None);
    }

    #[cfg(feature = "effect-meter")]
    #[test]
    fn effect_levels_audible_returns_none_with_empty_ring() {
        let player = test_player(vec![AudioEffect::Gain(GainEffect::default())]);
        player.set_effect_level_metering_enabled(true);
        assert_eq!(player.effect_levels_audible(), None);
    }

    #[cfg(feature = "effect-meter")]
    #[test]
    fn enabling_effect_level_metering_exposes_zeroed_slots() {
        let player = test_player(vec![AudioEffect::Gain(GainEffect::default())]);
        player.set_effect_level_metering_enabled(true);

        let snapshots = player.effect_levels().expect("level snapshots");
        let channels = player.info.channels.max(1) as usize;
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].input.peak, vec![0.0; channels]);
        assert_eq!(snapshots[0].output.rms, vec![0.0; channels]);
    }

    #[test]
    fn effect_frequency_responses_match_effect_slots() {
        let player = test_player(vec![
            AudioEffect::Gain(GainEffect::default()),
            AudioEffect::LowPassFilter(LowPassFilterEffect::default()),
        ]);

        let responses = player.effect_frequency_responses(32);

        assert_eq!(responses.len(), 2);
        assert!(responses[0].is_none());
        #[cfg(feature = "effect-meter")]
        assert_eq!(responses[1].as_ref().expect("response").composite.len(), 32);
        #[cfg(not(feature = "effect-meter"))]
        assert!(responses[1].is_none());
    }

    #[test]
    fn spectral_analysis_returns_none_while_disabled() {
        let player = test_player(vec![AudioEffect::LowPassFilter(
            LowPassFilterEffect::default(),
        )]);
        assert_eq!(player.effect_band_levels(), None);
        assert_eq!(player.effect_band_levels_audible(), None);
    }

    #[cfg(feature = "effect-meter-spectral")]
    #[test]
    fn enabling_spectral_analysis_exposes_zeroed_slots() {
        let player = test_player(vec![
            AudioEffect::Gain(GainEffect::default()),
            AudioEffect::LowPassFilter(LowPassFilterEffect::default()),
        ]);
        player.set_spectral_analysis_enabled(true);

        let snapshots = player.effect_band_levels().expect("spectral snapshots");
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0], None);
        assert_eq!(snapshots[1], None);
        assert_eq!(player.effect_band_levels_audible(), None);
    }

    #[cfg(feature = "effect-meter-spectral")]
    #[test]
    fn spectral_enable_accessor_tracks_runtime_state() {
        let player = test_player(vec![AudioEffect::LowPassFilter(
            LowPassFilterEffect::default(),
        )]);
        assert!(!player.spectral_analysis_enabled());

        player.set_spectral_analysis_enabled(true);
        assert!(player.spectral_analysis_enabled());

        player.set_spectral_analysis_enabled(false);
        assert!(!player.spectral_analysis_enabled());
    }

    fn test_player(effects: Vec<AudioEffect>) -> Player {
        let player = Player::new_from_file_paths(vec![PathsTrack::new_from_file_paths(vec![
            "/tmp/nonexistent.wav".to_string(),
        ])]);
        player.playback_thread_exists.store(false, Ordering::SeqCst);
        player.abort.store(true, Ordering::SeqCst);
        *player.lock_playback_thread_handle_invariant() = None;
        *player.lock_state_invariant() = PlayerState::Stopped;
        *player.lock_effects_recoverable() = effects.clone();
        player
            .effect_meter
            .set_level_layout_zeroed(effects.len(), player.info.channels as usize);
        player
            .effect_meter
            .set_spectral_layout_zeroed(effects.len());
        player
    }
}
