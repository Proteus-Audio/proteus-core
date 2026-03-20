#![cfg_attr(
    not(any(feature = "effect-meter", feature = "effect-meter-spectral")),
    allow(dead_code, unused_imports, unused_variables)
)]

//! Shared runtime store for per-effect inspection snapshots.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};

use crate::dsp::guardrails::{sanitize_channels, sanitize_finite_min};
use crate::dsp::meter::level::resize_level_snapshot;
use crate::dsp::meter::{EffectBandSnapshot, EffectLevelSnapshot};
use crate::playback::mutex_policy::lock_recoverable;

const DEFAULT_EFFECT_LEVEL_METER_REFRESH_HZ: f32 = 30.0;
#[cfg(feature = "effect-meter-spectral")]
const DEFAULT_SPECTRAL_ANALYSIS_REFRESH_HZ: f32 = 15.0;
const AUDIBLE_RING_CAPACITY: usize = 32;

/// A metering snapshot tagged with the mix-thread producer clock.
#[cfg(feature = "effect-meter")]
#[derive(Debug)]
struct TimestampedLevelSnapshot {
    /// Cumulative audio time at the mix→worker boundary for this snapshot.
    mix_time_secs: f64,
    /// Per-effect level measurements captured around the DSP chain.
    snapshots: Vec<EffectLevelSnapshot>,
}

/// Shared runtime meter state used by the control path and the mix thread.
#[derive(Debug)]
pub struct EffectMeter {
    level_enabled: AtomicBool,
    level_refresh_hz_bits: AtomicU32,
    level_snapshots: Mutex<Vec<EffectLevelSnapshot>>,
    #[cfg(feature = "effect-meter")]
    audible_ring: Mutex<VecDeque<TimestampedLevelSnapshot>>,
    #[cfg(feature = "effect-meter-spectral")]
    spectral_enabled: AtomicBool,
    #[cfg(feature = "effect-meter-spectral")]
    spectral_refresh_hz_bits: AtomicU32,
    #[cfg(feature = "effect-meter-spectral")]
    spectral_snapshots: Mutex<Vec<Option<EffectBandSnapshot>>>,
}

impl EffectMeter {
    pub(crate) fn new() -> Self {
        Self {
            level_enabled: AtomicBool::new(false),
            level_refresh_hz_bits: AtomicU32::new(DEFAULT_EFFECT_LEVEL_METER_REFRESH_HZ.to_bits()),
            level_snapshots: Mutex::new(Vec::new()),
            #[cfg(feature = "effect-meter")]
            audible_ring: Mutex::new(VecDeque::new()),
            #[cfg(feature = "effect-meter-spectral")]
            spectral_enabled: AtomicBool::new(false),
            #[cfg(feature = "effect-meter-spectral")]
            spectral_refresh_hz_bits: AtomicU32::new(
                DEFAULT_SPECTRAL_ANALYSIS_REFRESH_HZ.to_bits(),
            ),
            #[cfg(feature = "effect-meter-spectral")]
            spectral_snapshots: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn reset(&self) {
        lock_recoverable(
            &self.level_snapshots,
            "effect level snapshots",
            "effect level snapshots are derived telemetry that can be rebuilt",
        )
        .clear();
        #[cfg(feature = "effect-meter")]
        lock_recoverable(
            &self.audible_ring,
            "audible effect level ring",
            "audible-time snapshots are derived telemetry that can be rebuilt",
        )
        .clear();
        #[cfg(feature = "effect-meter-spectral")]
        lock_recoverable(
            &self.spectral_snapshots,
            "effect spectral snapshots",
            "effect spectral snapshots are derived telemetry that can be rebuilt",
        )
        .clear();
    }

    pub(crate) fn set_level_metering_enabled(&self, enabled: bool) {
        #[cfg(feature = "effect-meter")]
        self.level_enabled.store(enabled, Ordering::Relaxed);

        #[cfg(not(feature = "effect-meter"))]
        let _ = enabled;
    }

    pub(crate) fn level_metering_enabled(&self) -> bool {
        #[cfg(feature = "effect-meter")]
        {
            self.level_enabled.load(Ordering::Relaxed)
        }

        #[cfg(not(feature = "effect-meter"))]
        {
            false
        }
    }

    pub(crate) fn set_level_refresh_hz(&self, hz: f32) {
        self.level_refresh_hz_bits
            .store(sanitize_refresh_hz(hz).to_bits(), Ordering::Relaxed);
    }

    pub(crate) fn level_refresh_hz(&self) -> f32 {
        f32::from_bits(self.level_refresh_hz_bits.load(Ordering::Relaxed))
    }

    pub(crate) fn effect_levels(&self) -> Option<Vec<EffectLevelSnapshot>> {
        if !self.level_metering_enabled() {
            return None;
        }

        #[cfg(feature = "effect-meter")]
        {
            Some(
                lock_recoverable(
                    &self.level_snapshots,
                    "effect level snapshots",
                    "effect level snapshots are derived telemetry that can be rebuilt",
                )
                .clone(),
            )
        }

        #[cfg(not(feature = "effect-meter"))]
        {
            None
        }
    }

    pub(crate) fn try_publish_levels(&self, snapshots: &[EffectLevelSnapshot]) {
        #[cfg(feature = "effect-meter")]
        if let Some(mut latest) = try_lock_recoverable(
            &self.level_snapshots,
            "effect level snapshots",
            "effect level snapshots are derived telemetry that can be rebuilt",
        ) {
            sync_level_snapshots(&mut latest, snapshots);
        }
    }

    /// Push a timestamped snapshot into the audible-time ring buffer.
    ///
    /// Called by the mix thread after each metered chunk. The `mix_time_secs`
    /// is the cumulative producer-clock position at the mix→worker boundary.
    pub(crate) fn push_timestamped_levels(
        &self,
        mix_time_secs: f64,
        snapshots: &[EffectLevelSnapshot],
    ) {
        #[cfg(feature = "effect-meter")]
        if let Some(mut ring) = try_lock_recoverable(
            &self.audible_ring,
            "audible effect level ring",
            "audible-time snapshots are derived telemetry that can be rebuilt",
        ) {
            if ring.len() >= AUDIBLE_RING_CAPACITY {
                ring.pop_front();
            }
            ring.push_back(TimestampedLevelSnapshot {
                mix_time_secs,
                snapshots: snapshots.to_vec(),
            });
        }

        #[cfg(not(feature = "effect-meter"))]
        {
            let _ = mix_time_secs;
            let _ = snapshots;
        }
    }

    /// Return the snapshot aligned to the given audible playback time.
    ///
    /// Finds the most recent ring-buffer entry whose producer-clock timestamp
    /// is at or before `audible_time_secs` and retires older entries.
    ///
    /// Returns `None` when the `effect-meter` feature is not compiled in, when
    /// runtime level metering is disabled, or when no snapshot has been
    /// produced yet.
    pub(crate) fn effect_levels_audible(
        &self,
        audible_time_secs: f64,
    ) -> Option<Vec<EffectLevelSnapshot>> {
        if !self.level_metering_enabled() {
            return None;
        }

        #[cfg(feature = "effect-meter")]
        {
            let mut ring = lock_recoverable(
                &self.audible_ring,
                "audible effect level ring",
                "audible-time snapshots are derived telemetry that can be rebuilt",
            );
            // Drain entries whose successor is also at or before audible time.
            while ring.len() > 1 {
                if ring
                    .get(1)
                    .is_none_or(|next| next.mix_time_secs > audible_time_secs)
                {
                    break;
                }
                ring.pop_front();
            }
            ring.front().map(|entry| entry.snapshots.clone())
        }

        #[cfg(not(feature = "effect-meter"))]
        {
            let _ = audible_time_secs;
            None
        }
    }

    pub(crate) fn set_level_layout_zeroed(&self, effect_count: usize, channels: usize) {
        #[cfg(feature = "effect-meter")]
        {
            let channels = sanitize_channels(channels);
            let mut latest = lock_recoverable(
                &self.level_snapshots,
                "effect level snapshots",
                "effect level snapshots are derived telemetry that can be rebuilt",
            );
            latest.clear();
            latest.resize_with(effect_count, EffectLevelSnapshot::default);
            for snapshot in latest.iter_mut() {
                resize_level_snapshot(&mut snapshot.input, channels);
                resize_level_snapshot(&mut snapshot.output, channels);
                snapshot.input.peak.fill(0.0);
                snapshot.input.rms.fill(0.0);
                snapshot.output.peak.fill(0.0);
                snapshot.output.rms.fill(0.0);
            }
        }

        #[cfg(not(feature = "effect-meter"))]
        {
            let _ = effect_count;
            let _ = channels;
        }
    }

    pub(crate) fn set_spectral_analysis_enabled(&self, enabled: bool) {
        #[cfg(feature = "effect-meter-spectral")]
        self.spectral_enabled.store(enabled, Ordering::Relaxed);

        #[cfg(not(feature = "effect-meter-spectral"))]
        let _ = enabled;
    }

    pub(crate) fn spectral_analysis_enabled(&self) -> bool {
        #[cfg(feature = "effect-meter-spectral")]
        {
            self.spectral_enabled.load(Ordering::Relaxed)
        }

        #[cfg(not(feature = "effect-meter-spectral"))]
        {
            false
        }
    }

    pub(crate) fn set_spectral_refresh_hz(&self, hz: f32) {
        #[cfg(feature = "effect-meter-spectral")]
        self.spectral_refresh_hz_bits
            .store(sanitize_refresh_hz(hz).to_bits(), Ordering::Relaxed);

        #[cfg(not(feature = "effect-meter-spectral"))]
        let _ = hz;
    }

    #[cfg(feature = "effect-meter-spectral")]
    pub(crate) fn spectral_refresh_hz(&self) -> f32 {
        f32::from_bits(self.spectral_refresh_hz_bits.load(Ordering::Relaxed))
    }

    pub(crate) fn effect_band_levels(&self) -> Option<Vec<Option<EffectBandSnapshot>>> {
        if !self.spectral_analysis_enabled() {
            return None;
        }

        #[cfg(feature = "effect-meter-spectral")]
        {
            Some(
                lock_recoverable(
                    &self.spectral_snapshots,
                    "effect spectral snapshots",
                    "effect spectral snapshots are derived telemetry that can be rebuilt",
                )
                .clone(),
            )
        }

        #[cfg(not(feature = "effect-meter-spectral"))]
        {
            None
        }
    }

    #[cfg(feature = "effect-meter-spectral")]
    pub(crate) fn try_publish_spectral(&self, snapshots: &[Option<EffectBandSnapshot>]) {
        if let Some(mut latest) = try_lock_recoverable(
            &self.spectral_snapshots,
            "effect spectral snapshots",
            "effect spectral snapshots are derived telemetry that can be rebuilt",
        ) {
            latest.clear();
            latest.extend_from_slice(snapshots);
        }
    }

    pub(crate) fn set_spectral_layout_zeroed(&self, effect_count: usize) {
        #[cfg(feature = "effect-meter-spectral")]
        {
            let mut latest = lock_recoverable(
                &self.spectral_snapshots,
                "effect spectral snapshots",
                "effect spectral snapshots are derived telemetry that can be rebuilt",
            );
            latest.clear();
            latest.resize(effect_count, None);
        }

        #[cfg(not(feature = "effect-meter-spectral"))]
        let _ = effect_count;
    }
}

fn sanitize_refresh_hz(hz: f32) -> f32 {
    sanitize_finite_min(hz, 1.0, 1.0)
}

fn sync_level_snapshots(target: &mut Vec<EffectLevelSnapshot>, source: &[EffectLevelSnapshot]) {
    target.resize_with(source.len(), EffectLevelSnapshot::default);
    for (target_snapshot, source_snapshot) in target.iter_mut().zip(source.iter()) {
        resize_level_snapshot(&mut target_snapshot.input, source_snapshot.input.peak.len());
        resize_level_snapshot(
            &mut target_snapshot.output,
            source_snapshot.output.peak.len(),
        );
        target_snapshot
            .input
            .peak
            .copy_from_slice(&source_snapshot.input.peak);
        target_snapshot
            .input
            .rms
            .copy_from_slice(&source_snapshot.input.rms);
        target_snapshot
            .output
            .peak
            .copy_from_slice(&source_snapshot.output.peak);
        target_snapshot
            .output
            .rms
            .copy_from_slice(&source_snapshot.output.rms);
    }
}

fn try_lock_recoverable<'a, T>(
    mutex: &'a Mutex<T>,
    label: &str,
    rationale: &str,
) -> Option<MutexGuard<'a, T>> {
    match mutex.try_lock() {
        Ok(guard) => Some(guard),
        Err(TryLockError::WouldBlock) => None,
        Err(TryLockError::Poisoned(err)) => {
            log::warn!(
                "{label} lock poisoned; recovering with the inner value because {rationale}"
            );
            Some(err.into_inner())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_snapshot(peak: f32) -> EffectLevelSnapshot {
        EffectLevelSnapshot {
            input: crate::dsp::meter::LevelSnapshot {
                peak: vec![peak],
                rms: vec![peak * 0.7],
            },
            output: crate::dsp::meter::LevelSnapshot {
                peak: vec![peak * 0.9],
                rms: vec![peak * 0.6],
            },
        }
    }

    #[test]
    fn audible_returns_none_when_metering_disabled() {
        let meter = EffectMeter::new();
        assert_eq!(meter.effect_levels_audible(0.0), None);
    }

    #[cfg(feature = "effect-meter")]
    #[test]
    fn audible_returns_none_when_ring_is_empty() {
        let meter = EffectMeter::new();
        meter.set_level_metering_enabled(true);
        assert_eq!(meter.effect_levels_audible(0.0), None);
    }

    #[cfg(feature = "effect-meter")]
    #[test]
    fn audible_returns_snapshot_at_or_before_audible_time() {
        let meter = EffectMeter::new();
        meter.set_level_metering_enabled(true);

        meter.push_timestamped_levels(1.0, &[make_snapshot(0.1)]);
        meter.push_timestamped_levels(2.0, &[make_snapshot(0.5)]);
        meter.push_timestamped_levels(3.0, &[make_snapshot(0.9)]);

        // Audible time between first and second → returns first.
        let result = meter.effect_levels_audible(1.5).expect("snapshot");
        assert_eq!(result.len(), 1);
        assert!((result[0].input.peak[0] - 0.1).abs() < 1e-6);

        // Audible time exactly at second → returns second.
        let result = meter.effect_levels_audible(2.0).expect("snapshot");
        assert!((result[0].input.peak[0] - 0.5).abs() < 1e-6);

        // Audible time past all entries → returns last.
        let result = meter.effect_levels_audible(5.0).expect("snapshot");
        assert!((result[0].input.peak[0] - 0.9).abs() < 1e-6);
    }

    #[cfg(feature = "effect-meter")]
    #[test]
    fn audible_drains_old_entries() {
        let meter = EffectMeter::new();
        meter.set_level_metering_enabled(true);

        meter.push_timestamped_levels(1.0, &[make_snapshot(0.1)]);
        meter.push_timestamped_levels(2.0, &[make_snapshot(0.5)]);
        meter.push_timestamped_levels(3.0, &[make_snapshot(0.9)]);

        // Query at 2.5 should drain the 1.0 entry, keeping 2.0 and 3.0.
        let _ = meter.effect_levels_audible(2.5);
        let ring = meter.audible_ring.lock().unwrap();
        assert_eq!(ring.len(), 2);
        assert!((ring[0].mix_time_secs - 2.0).abs() < 1e-9);
    }

    #[cfg(feature = "effect-meter")]
    #[test]
    fn ring_buffer_respects_capacity_bound() {
        let meter = EffectMeter::new();
        meter.set_level_metering_enabled(true);

        for i in 0..(AUDIBLE_RING_CAPACITY + 10) {
            meter.push_timestamped_levels(i as f64, &[make_snapshot(0.1)]);
        }

        let ring = meter.audible_ring.lock().unwrap();
        assert!(ring.len() <= AUDIBLE_RING_CAPACITY);
    }

    #[cfg(feature = "effect-meter")]
    #[test]
    fn reset_clears_audible_ring() {
        let meter = EffectMeter::new();
        meter.set_level_metering_enabled(true);
        meter.push_timestamped_levels(1.0, &[make_snapshot(0.5)]);
        assert!(meter.audible_ring.lock().unwrap().len() > 0);

        meter.reset();
        assert!(meter.audible_ring.lock().unwrap().is_empty());
    }

    #[test]
    fn push_timestamped_is_noop_without_feature() {
        let meter = EffectMeter::new();
        // Should not panic even when metering is disabled.
        meter.push_timestamped_levels(1.0, &[make_snapshot(0.5)]);
    }
}
