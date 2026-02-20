//! Schedule-driven source router and logical-track mixer.

use std::collections::{HashMap, HashSet};

use dasp_ring_buffer::Bounded;

use crate::container::prot::{RuntimeInstanceMeta, RuntimeInstancePlan, ShuffleSource};

use super::track_stage::{apply_track_gain_pan, combine_tracks_equal_weight};

/// Source identifier used by decode workers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum SourceKey {
    /// Container track id source.
    TrackId(u32),
    /// Standalone file path source.
    FilePath(String),
}

impl From<&ShuffleSource> for SourceKey {
    fn from(value: &ShuffleSource) -> Self {
        match value {
            ShuffleSource::TrackId(track_id) => Self::TrackId(*track_id),
            ShuffleSource::FilePath(path) => Self::FilePath(path.clone()),
        }
    }
}

/// Aggregate fill state for a track or the whole mix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FillState {
    /// Mix/track buffers are neither uniformly full nor uniformly not-full.
    Partial,
    /// Every instance currently reports full.
    Full,
    /// No instance currently reports full.
    NotFull,
}

/// Debug telemetry returned by routing calls.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RouteDecision {
    /// Instance ids that received decoded source samples.
    pub(crate) sample_targets_written: Vec<usize>,
    /// Instance ids that received zero-fill for this packet span.
    pub(crate) zero_fill_targets_written: Vec<usize>,
    /// True when no instance was relevant for this packet.
    pub(crate) ignored: bool,
}

#[derive(Debug)]
struct BufferInstance {
    meta: RuntimeInstanceMeta,
    buffer: Bounded<Vec<f32>>,
    full: bool,
    finished: bool,
    produced_samples: u64,
    zero_filled_samples: u64,
    eof_reached_ms: Option<u64>,
}

impl BufferInstance {
    fn new(meta: RuntimeInstanceMeta, capacity_samples: usize) -> Self {
        Self {
            meta,
            buffer: Bounded::from(vec![0.0_f32; capacity_samples.max(1)]),
            full: false,
            finished: false,
            produced_samples: 0,
            zero_filled_samples: 0,
            eof_reached_ms: None,
        }
    }
}

/// Router/mixer that owns per-instance buffers and schedule-window alignment.
#[derive(Debug)]
pub(crate) struct BufferMixer {
    sample_rate: u32,
    channels: usize,
    mix_chunk_samples: usize,
    consumed_samples: usize,
    instances: Vec<BufferInstance>,
    track_instances: Vec<Vec<usize>>,
    track_mix_settings: HashMap<usize, (f32, f32)>,
    slot_to_logical: HashMap<usize, usize>,
}

impl BufferMixer {
    /// Create a new buffer mixer from a runtime instance plan.
    pub(crate) fn new(
        plan: RuntimeInstancePlan,
        sample_rate: u32,
        channels: usize,
        capacity_samples: usize,
        track_mix_settings: HashMap<usize, (f32, f32)>,
        mix_chunk_samples: usize,
    ) -> Self {
        let mut instances = Vec::with_capacity(plan.instances.len());
        let mut slot_to_logical = HashMap::new();
        for meta in plan.instances {
            slot_to_logical.insert(meta.slot_index, meta.logical_track_index);
            instances.push(BufferInstance::new(meta, capacity_samples));
        }

        let mut track_instances = vec![Vec::new(); plan.logical_track_count];
        for (index, instance) in instances.iter().enumerate() {
            if instance.meta.logical_track_index >= track_instances.len() {
                track_instances.resize(instance.meta.logical_track_index + 1, Vec::new());
            }
            track_instances[instance.meta.logical_track_index].push(index);
        }

        Self {
            sample_rate: sample_rate.max(1),
            channels: channels.max(1),
            mix_chunk_samples: mix_chunk_samples.max(1),
            consumed_samples: 0,
            instances,
            track_instances,
            track_mix_settings,
            slot_to_logical,
        }
    }

    /// Route one decoded packet into schedule-owned instance buffers.
    pub(crate) fn route_packet(
        &mut self,
        samples: &[f32],
        source: SourceKey,
        packet_ts: f64,
    ) -> RouteDecision {
        if samples.is_empty() {
            return RouteDecision {
                ignored: true,
                ..RouteDecision::default()
            };
        }

        let frame_count = samples.len() / self.channels;
        if frame_count == 0 {
            return RouteDecision {
                ignored: true,
                ..RouteDecision::default()
            };
        }

        let mut decision = RouteDecision::default();
        for instance in self.instances.iter_mut() {
            let overlap = packet_overlap_samples(
                packet_ts,
                frame_count,
                self.sample_rate,
                self.channels,
                &instance.meta.active_windows,
            );
            if overlap.is_empty() {
                continue;
            }

            let source_match = SourceKey::from(&instance.meta.source_key) == source;
            let mut wrote_any = false;
            for (start_sample, end_sample) in overlap {
                if start_sample >= end_sample || end_sample > samples.len() {
                    continue;
                }

                if source_match {
                    wrote_any |= push_slice(
                        &mut instance.buffer,
                        &samples[start_sample..end_sample],
                        &mut instance.full,
                    );
                    instance.produced_samples = instance
                        .produced_samples
                        .saturating_add((end_sample - start_sample) as u64);
                } else {
                    let zeros = vec![0.0_f32; end_sample - start_sample];
                    wrote_any |= push_slice(&mut instance.buffer, &zeros, &mut instance.full);
                    instance.zero_filled_samples = instance
                        .zero_filled_samples
                        .saturating_add((end_sample - start_sample) as u64);
                }
            }

            if wrote_any {
                if source_match {
                    decision
                        .sample_targets_written
                        .push(instance.meta.instance_id);
                } else {
                    decision
                        .zero_fill_targets_written
                        .push(instance.meta.instance_id);
                }
            }
        }

        decision.ignored = decision.sample_targets_written.is_empty()
            && decision.zero_fill_targets_written.is_empty();
        decision
    }

    /// Mark all instances for `source_key` as finished.
    pub(crate) fn signal_finish(&mut self, source_key: &SourceKey) {
        let eof_ms = samples_to_ms(self.consumed_samples, self.sample_rate, self.channels);
        for instance in self.instances.iter_mut() {
            if SourceKey::from(&instance.meta.source_key) != *source_key {
                continue;
            }
            if !instance.finished {
                instance.finished = true;
                instance.eof_reached_ms = Some(eof_ms);
            }
        }
    }

    /// True when each instance in the logical track has samples or is finished.
    pub(crate) fn track_ready(&self, logical_track_index: usize) -> bool {
        self.track_ready_with_min_samples(logical_track_index, 1)
    }

    /// True when each instance in the logical track has at least `min_samples`
    /// available (or is finished/not currently active).
    pub(crate) fn track_ready_with_min_samples(
        &self,
        logical_track_index: usize,
        min_samples: usize,
    ) -> bool {
        let Some(instances) = self.track_instances.get(logical_track_index) else {
            return true;
        };

        instances.iter().all(|instance_index| {
            let instance = &self.instances[*instance_index];
            if !instance_needs_data(
                instance,
                self.consumed_samples,
                self.sample_rate,
                self.channels,
            ) {
                return true;
            }
            instance.finished || instance.buffer.len() >= min_samples.max(1)
        })
    }

    /// True when each instance in the logical track is marked finished.
    pub(crate) fn track_finished(&self, logical_track_index: usize) -> bool {
        let Some(instances) = self.track_instances.get(logical_track_index) else {
            return true;
        };
        instances.iter().all(|instance_index| {
            let instance = &self.instances[*instance_index];
            instance.finished
                || instance_fully_past_window(
                    instance,
                    self.consumed_samples,
                    self.sample_rate,
                    self.channels,
                )
        })
    }

    /// True when all logical tracks are ready.
    pub(crate) fn mix_ready(&self) -> bool {
        self.mix_ready_with_min_samples(1)
    }

    /// True when all logical tracks are ready for at least `min_samples`.
    pub(crate) fn mix_ready_with_min_samples(&self, min_samples: usize) -> bool {
        (0..self.track_instances.len())
            .all(|track_index| self.track_ready_with_min_samples(track_index, min_samples))
    }

    /// True when all logical tracks are finished.
    pub(crate) fn mix_finished(&self) -> bool {
        (0..self.track_instances.len()).all(|track_index| self.track_finished(track_index))
    }

    /// Fill-state aggregate for one logical track.
    pub(crate) fn track_fill_state(&self, logical_track_index: usize) -> FillState {
        let Some(instances) = self.track_instances.get(logical_track_index) else {
            return FillState::NotFull;
        };
        aggregate_fill_state(
            instances
                .iter()
                .map(|instance_index| self.instances[*instance_index].full),
        )
    }

    /// Fill-state aggregate across all logical tracks.
    pub(crate) fn mix_fill_state(&self) -> FillState {
        aggregate_fill_state(self.instances.iter().map(|instance| instance.full))
    }

    /// Take synchronized mixed samples across all logical tracks.
    pub(crate) fn take_samples(&mut self) -> Option<Vec<f32>> {
        if !self.mix_ready() {
            return None;
        }

        let mut ready_samples_per_track = Vec::with_capacity(self.track_instances.len());
        for track_indices in &self.track_instances {
            let mut track_min = usize::MAX;
            for instance_index in track_indices {
                let instance = &self.instances[*instance_index];
                let available = if !instance_needs_data(
                    instance,
                    self.consumed_samples,
                    self.sample_rate,
                    self.channels,
                ) || instance_fully_past_window(
                    instance,
                    self.consumed_samples,
                    self.sample_rate,
                    self.channels,
                ) {
                    usize::MAX
                } else if instance.buffer.len() > 0 {
                    instance.buffer.len()
                } else if instance.finished {
                    usize::MAX
                } else {
                    0
                };
                track_min = track_min.min(available);
            }
            ready_samples_per_track.push(track_min);
        }

        let to_consume = ready_samples_per_track
            .into_iter()
            .min()
            .unwrap_or(0)
            .min(self.mix_chunk_samples);
        if to_consume == 0 || to_consume == usize::MAX {
            return None;
        }

        let mut logical_tracks = Vec::with_capacity(self.track_instances.len());
        for (track_index, track_indices) in self.track_instances.iter().enumerate() {
            let mut track_buffer = vec![0.0_f32; to_consume];

            for instance_index in track_indices {
                let instance = &mut self.instances[*instance_index];
                if !instance_needs_data(
                    instance,
                    self.consumed_samples,
                    self.sample_rate,
                    self.channels,
                ) || instance_fully_past_window(
                    instance,
                    self.consumed_samples,
                    self.sample_rate,
                    self.channels,
                ) {
                    continue;
                }
                for sample in track_buffer.iter_mut().take(to_consume) {
                    if let Some(value) = instance.buffer.pop() {
                        *sample += value;
                    }
                }
            }

            let (level, pan) = self
                .track_mix_settings
                .get(&track_index)
                .copied()
                .unwrap_or((1.0, 0.0));
            apply_track_gain_pan(&mut track_buffer, level, pan, self.channels);
            logical_tracks.push(track_buffer);
        }

        self.consumed_samples = self.consumed_samples.saturating_add(to_consume);
        Some(combine_tracks_equal_weight(&logical_tracks))
    }

    /// Return per-instance debug counters.
    pub(crate) fn counters(&self) -> Vec<(usize, u64, u64, Option<u64>)> {
        self.instances
            .iter()
            .map(|instance| {
                (
                    instance.meta.instance_id,
                    instance.produced_samples,
                    instance.zero_filled_samples,
                    instance.eof_reached_ms,
                )
            })
            .collect()
    }

    /// Return unique source keys referenced by this runtime plan.
    pub(crate) fn sources(&self) -> Vec<SourceKey> {
        let mut set = HashSet::new();
        for instance in self.instances.iter() {
            set.insert(SourceKey::from(&instance.meta.source_key));
        }
        set.into_iter().collect()
    }

    /// Number of concrete instances in the mixer.
    pub(crate) fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// Number of logical tracks represented by the plan.
    pub(crate) fn logical_track_count(&self) -> usize {
        self.track_instances.len()
    }

    /// Count instances that are finished or fully elapsed.
    pub(crate) fn finished_instance_count(&self) -> usize {
        self.instances
            .iter()
            .filter(|instance| {
                instance.finished
                    || instance_fully_past_window(
                        instance,
                        self.consumed_samples,
                        self.sample_rate,
                        self.channels,
                    )
            })
            .count()
    }

    /// Update per-track mix controls using a slot index.
    pub(crate) fn set_track_mix_by_slot(&mut self, slot_index: usize, level: f32, pan: f32) {
        if let Some(logical_track_index) = self.slot_to_logical.get(&slot_index).copied() {
            self.track_mix_settings
                .insert(logical_track_index, (level.max(0.0), pan.clamp(-1.0, 1.0)));
        }
    }
}

fn push_slice(buffer: &mut Bounded<Vec<f32>>, slice: &[f32], full_flag: &mut bool) -> bool {
    let mut wrote = false;
    let mut overflow = false;
    for sample in slice.iter().copied() {
        if buffer.len() >= buffer.max_len() {
            overflow = true;
            break;
        }
        buffer.push(sample);
        wrote = true;
    }
    *full_flag = overflow;
    wrote
}

fn aggregate_fill_state<I>(states: I) -> FillState
where
    I: IntoIterator<Item = bool>,
{
    let mut saw = false;
    let mut all_full = true;
    let mut any_full = false;

    for full in states {
        saw = true;
        all_full &= full;
        any_full |= full;
    }

    if !saw || !any_full {
        FillState::NotFull
    } else if all_full {
        FillState::Full
    } else {
        FillState::Partial
    }
}

fn packet_overlap_samples(
    packet_ts: f64,
    frame_count: usize,
    sample_rate: u32,
    channels: usize,
    windows: &[crate::container::prot::ActiveWindow],
) -> Vec<(usize, usize)> {
    let packet_start = packet_ts.max(0.0);
    let packet_end = packet_start + (frame_count as f64 / sample_rate as f64);
    let mut spans = Vec::new();
    for window in windows {
        let window_start = window.start_ms as f64 / 1000.0;
        let window_end = window
            .end_ms
            .map(|end| end as f64 / 1000.0)
            .unwrap_or(f64::INFINITY);

        let overlap_start = packet_start.max(window_start);
        let overlap_end = packet_end.min(window_end);
        if overlap_start >= overlap_end {
            continue;
        }

        let start_frame = (((overlap_start - packet_start) * sample_rate as f64).floor() as usize)
            .min(frame_count);
        let end_frame =
            (((overlap_end - packet_start) * sample_rate as f64).ceil() as usize).min(frame_count);
        if end_frame <= start_frame {
            continue;
        }

        spans.push((start_frame * channels, end_frame * channels));
    }
    spans
}

fn instance_needs_data(
    instance: &BufferInstance,
    consumed_samples: usize,
    sample_rate: u32,
    channels: usize,
) -> bool {
    let start_sample = window_start_samples(instance, sample_rate, channels);
    let end_sample = window_end_samples(instance, sample_rate, channels);
    consumed_samples >= start_sample && end_sample.map(|end| consumed_samples < end).unwrap_or(true)
}

fn instance_fully_past_window(
    instance: &BufferInstance,
    consumed_samples: usize,
    sample_rate: u32,
    channels: usize,
) -> bool {
    let Some(end_sample) = window_end_samples(instance, sample_rate, channels) else {
        return false;
    };
    consumed_samples >= end_sample && instance.buffer.len() == 0
}

fn window_start_samples(instance: &BufferInstance, sample_rate: u32, channels: usize) -> usize {
    let start_ms = instance
        .meta
        .active_windows
        .first()
        .map(|window| window.start_ms)
        .unwrap_or(0);
    ms_to_samples(start_ms, sample_rate, channels)
}

fn window_end_samples(
    instance: &BufferInstance,
    sample_rate: u32,
    channels: usize,
) -> Option<usize> {
    let end_ms = instance
        .meta
        .active_windows
        .last()
        .and_then(|window| window.end_ms);
    end_ms.map(|ms| ms_to_samples(ms, sample_rate, channels))
}

fn ms_to_samples(ms: u64, sample_rate: u32, channels: usize) -> usize {
    let frames = ((ms as f64 / 1000.0) * sample_rate as f64).round() as usize;
    frames.saturating_mul(channels)
}

fn samples_to_ms(samples: usize, sample_rate: u32, channels: usize) -> u64 {
    let frames = samples / channels.max(1);
    ((frames as f64 / sample_rate.max(1) as f64) * 1000.0).round() as u64
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::container::prot::{
        ActiveWindow, RuntimeInstanceMeta, RuntimeInstancePlan, ShuffleSource,
    };

    use super::{BufferMixer, FillState, SourceKey};

    fn simple_plan() -> RuntimeInstancePlan {
        RuntimeInstancePlan {
            logical_track_count: 2,
            instances: vec![
                RuntimeInstanceMeta {
                    instance_id: 0,
                    logical_track_index: 0,
                    slot_index: 0,
                    source_key: ShuffleSource::TrackId(1),
                    active_windows: vec![ActiveWindow {
                        start_ms: 0,
                        end_ms: Some(1000),
                    }],
                    selection_index: 0,
                    occurrence_index: 0,
                },
                RuntimeInstanceMeta {
                    instance_id: 1,
                    logical_track_index: 1,
                    slot_index: 1,
                    source_key: ShuffleSource::TrackId(2),
                    active_windows: vec![ActiveWindow {
                        start_ms: 0,
                        end_ms: Some(1000),
                    }],
                    selection_index: 0,
                    occurrence_index: 0,
                },
            ],
            event_boundaries_ms: vec![0],
        }
    }

    #[test]
    fn route_packet_targets_and_zero_fills_instances() {
        let mut mixer = BufferMixer::new(simple_plan(), 48_000, 2, 8, HashMap::new(), 4);

        let decision = mixer.route_packet(&[1.0, 1.0, 0.5, 0.5], SourceKey::TrackId(1), 0.0);
        assert_eq!(decision.sample_targets_written, vec![0]);
        assert_eq!(decision.zero_fill_targets_written, vec![1]);
        assert!(!decision.ignored);
    }

    #[test]
    fn readiness_and_take_samples_are_synchronized() {
        let mut mixer = BufferMixer::new(simple_plan(), 48_000, 2, 16, HashMap::new(), 4);

        mixer.route_packet(&[1.0, 1.0, 1.0, 1.0], SourceKey::TrackId(1), 0.0);
        assert!(mixer.mix_ready());
        let first = mixer.take_samples().expect("first mixed samples");
        assert_eq!(first, vec![0.5, 0.5, 0.5, 0.5]);

        mixer.route_packet(&[0.5, 0.5, 0.5, 0.5], SourceKey::TrackId(2), 0.0);
        assert!(mixer.mix_ready());

        let mixed = mixer.take_samples().expect("mixed samples");
        assert_eq!(mixed.len(), 4);
        assert_eq!(mixed, vec![0.25, 0.25, 0.25, 0.25]);
    }

    #[test]
    fn signal_finish_propagates_track_and_mix_finished() {
        let mut mixer = BufferMixer::new(simple_plan(), 48_000, 2, 8, HashMap::new(), 4);
        mixer.signal_finish(&SourceKey::TrackId(1));
        assert!(mixer.track_finished(0));
        assert!(!mixer.mix_finished());

        mixer.signal_finish(&SourceKey::TrackId(2));
        assert!(mixer.track_finished(1));
        assert!(mixer.mix_finished());
    }

    #[test]
    fn fill_state_aggregates_as_expected() {
        let mut track_mix = HashMap::new();
        track_mix.insert(0usize, (1.0_f32, 0.0_f32));
        let mut mixer = BufferMixer::new(simple_plan(), 48_000, 2, 2, track_mix, 4);
        assert_eq!(mixer.mix_fill_state(), FillState::NotFull);

        let _ = mixer.route_packet(&[1.0, 1.0, 1.0, 1.0], SourceKey::TrackId(1), 0.0);
        assert!(matches!(
            mixer.mix_fill_state(),
            FillState::Partial | FillState::Full
        ));
    }
}
