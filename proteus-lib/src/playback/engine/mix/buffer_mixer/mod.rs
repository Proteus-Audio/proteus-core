//! Schedule-driven source router and logical-track mixer.

mod aligned_buffer;
mod backpressure;
mod helpers;

use log::{debug, info, warn};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::dsp::utils::fade_interleaved_per_frame;
#[cfg(feature = "buffer-map")]
use crate::logging::clear_logfile;
use crate::playback::engine::mix::utils::TransitionDirection;
use crate::{
    container::prot::{RuntimeInstanceMeta, RuntimeInstancePlan, ShuffleSource},
    playback::engine::mix::utils::{map_cover, Cover},
};

use super::track_stage::{apply_track_gain_pan, combine_tracks_equal_weight};
use aligned_buffer::AlignedSampleBuffer;
pub(crate) use backpressure::DecodeBackpressure;
use helpers::{
    instance_fully_past_window, instance_needs_data, instance_past_window_ts, packet_overlap_samples,
    push_owned_slice, push_slice, push_zeros, samples_to_ms,
};
#[cfg(any(test, feature = "debug"))]
use helpers::aggregate_fill_state;
#[cfg(feature = "buffer-map")]
use helpers::{log_buffer, log_buffer_header};

/// Source identifier used by decode workers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum SourceKey {
    /// Container track id source.
    TrackId(u32),
    /// Standalone file path source.
    FilePath(String),
}

impl From<&ShuffleSource> for SourceKey {
    /// Convert a runtime shuffle source into a decode-worker source key.
    fn from(value: &ShuffleSource) -> Self {
        match value {
            ShuffleSource::TrackId(track_id) => Self::TrackId(*track_id),
            ShuffleSource::FilePath(path) => Self::FilePath(path.clone()),
        }
    }
}

/// Aggregate fill state for a track or the whole mix.
#[cfg(any(test, feature = "debug"))]
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
    buffer: AlignedSampleBuffer,
    buffer_capacity_samples: usize,
    full: bool,
    finished: bool,
    produced_samples: u64,
    zero_filled_samples: u64,
    eof_reached_ms: Option<u64>,
}

impl BufferInstance {
    /// Create an empty per-instance buffer and counters for one runtime instance.
    fn new(meta: RuntimeInstanceMeta, capacity_samples: usize) -> Self {
        Self {
            meta,
            buffer: AlignedSampleBuffer::with_capacity(capacity_samples.max(1)),
            buffer_capacity_samples: capacity_samples.max(1),
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
    decode_backpressure: Arc<DecodeBackpressure>,
    crossfade_ms: usize,
    pop_warning: Vec<usize>,
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
        #[cfg(feature = "buffer-map")]
        clear_logfile();
        // println!("Shuffle plan: {:?}", plan);
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

        let decode_backpressure = Arc::new(DecodeBackpressure::from_instances(&instances));

        Self {
            sample_rate: sample_rate.max(1),
            channels: channels.max(1),
            mix_chunk_samples: mix_chunk_samples.max(1),
            consumed_samples: 0,
            instances,
            track_instances,
            track_mix_settings,
            slot_to_logical,
            decode_backpressure,
            crossfade_ms: 2,
            pop_warning: Vec::new(),
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
        for (instance_index, instance) in self.instances.iter_mut().enumerate() {
            if instance.finished {
                continue;
            }

            if SourceKey::from(&instance.meta.source_key) != source {
                continue;
            }

            if instance_past_window_ts(instance, &packet_ts) {
                debug!(
                    "Instance {} (Track {}) is finished!!",
                    instance.meta.instance_id, instance.meta.logical_track_index
                );
                instance.finished = true;
                self.decode_backpressure.on_finished(instance_index);
                continue;
            }

            let overlap = packet_overlap_samples(
                packet_ts,
                frame_count,
                self.sample_rate,
                self.channels,
                &instance.meta.active_windows,
            );

            let cover_transition = self.crossfade_ms * self.sample_rate as usize / 1000;
            let cover = map_cover(&overlap, samples.len(), Some(cover_transition));

            // if last_of_window || (first_window_start > 0.0 && first_window_start > packet_ts) {
            debug!(
                "Instance {} / Track {} / Time {} / Overlap {:?} / Cover {:?}",
                instance.meta.instance_id,
                instance.meta.logical_track_index,
                // samples.len(),
                packet_ts,
                overlap,
                cover,
                // first_window.unwrap()
            );
            // }

            let mut wrote_real = false;
            let mut wrote_zero = false;
            for section in cover {
                match section {
                    Cover::Overlap((start_sample, end_sample)) => {
                        if start_sample >= end_sample || end_sample > samples.len() {
                            continue;
                        }

                        let push = push_slice(
                            &mut instance.buffer,
                            instance.buffer_capacity_samples,
                            &samples[start_sample..end_sample],
                            &mut instance.full,
                        );
                        self.decode_backpressure.on_samples_pushed(
                            instance_index,
                            end_sample - start_sample,
                            push.written_samples,
                            instance.full,
                        );
                        if push.wrote_any {
                            wrote_real = true;
                        }
                        if push.written_samples < (end_sample - start_sample) {
                            warn!(
                                "Partial overlap write for i{}: wrote {} / {} samples",
                                instance.meta.instance_id,
                                push.written_samples,
                                end_sample - start_sample
                            );
                        }
                        instance.produced_samples = instance
                            .produced_samples
                            .saturating_add(push.written_samples as u64);
                    }
                    Cover::Underlay((start_sample, end_sample)) => {
                        let length = end_sample - start_sample;

                        let push = push_zeros(
                            &mut instance.buffer,
                            instance.buffer_capacity_samples,
                            length,
                            &mut instance.full,
                        );
                        self.decode_backpressure.on_samples_pushed(
                            instance_index,
                            length,
                            push.written_samples,
                            instance.full,
                        );
                        if push.wrote_any {
                            wrote_zero = true;
                        }
                        if push.written_samples < length {
                            warn!(
                                "Partial underlay write for i{}: wrote {} / {} samples",
                                instance.meta.instance_id, push.written_samples, length
                            );
                        }
                        instance.zero_filled_samples = instance
                            .zero_filled_samples
                            .saturating_add(push.written_samples as u64);
                    }

                    Cover::Transition((direction, (start_sample, end_sample))) => {
                        if start_sample >= end_sample || end_sample > samples.len() {
                            continue;
                        }

                        let slice_length = end_sample - start_sample;

                        info!(
                            "Transition starting at: {}",
                            packet_ts
                                + (samples_to_ms(start_sample, self.sample_rate, self.channels)
                                    as f64
                                    / 1000.0)
                        );

                        let (ramp_start, ramp_end) = match direction {
                            TransitionDirection::Up => {
                                let starting_val = (cover_transition as f32 - slice_length as f32)
                                    / cover_transition as f32;

                                (starting_val, 1.0)
                            }
                            TransitionDirection::Down => {
                                let ending_val = (cover_transition as f32 - slice_length as f32)
                                    / cover_transition as f32;

                                (1.0, ending_val)
                            }
                        };

                        info!("Ramp: {:?}", (ramp_start, ramp_end));

                        let mut slice = samples[start_sample..end_sample].to_vec();

                        fade_interleaved_per_frame(&mut slice, self.channels, ramp_start, ramp_end);

                        let push = push_owned_slice(
                            &mut instance.buffer,
                            instance.buffer_capacity_samples,
                            slice,
                            &mut instance.full,
                        );
                        self.decode_backpressure.on_samples_pushed(
                            instance_index,
                            slice_length,
                            push.written_samples,
                            instance.full,
                        );
                        if push.wrote_any {
                            wrote_real = true;
                        }
                        if push.written_samples < slice_length {
                            warn!(
                                "Partial transition write for i{}: wrote {} / {} samples",
                                instance.meta.instance_id, push.written_samples, slice_length
                            );
                        }
                        instance.produced_samples = instance
                            .produced_samples
                            .saturating_add(push.written_samples as u64);
                    }
                }
            }

            if wrote_real {
                decision
                    .sample_targets_written
                    .push(instance.meta.instance_id);
            }
            if wrote_zero {
                decision
                    .zero_fill_targets_written
                    .push(instance.meta.instance_id);
            }
        }

        decision.ignored = decision.sample_targets_written.is_empty()
            && decision.zero_fill_targets_written.is_empty();
        decision
    }

    /// Mark all instances for `source_key` as finished.
    pub(crate) fn signal_finish(&mut self, source_key: &SourceKey) {
        let eof_ms = samples_to_ms(self.consumed_samples, self.sample_rate, self.channels);
        for (instance_index, instance) in self.instances.iter_mut().enumerate() {
            if SourceKey::from(&instance.meta.source_key) != *source_key {
                continue;
            }
            if !instance.finished {
                instance.finished = true;
                instance.eof_reached_ms = Some(eof_ms);
                self.decode_backpressure.on_finished(instance_index);
            }
        }
    }

    /// Mark all instances for `source_key` as finished.
    pub(crate) fn signal_finish_all(&mut self) {
        let eof_ms = samples_to_ms(self.consumed_samples, self.sample_rate, self.channels);
        for (instance_index, instance) in self.instances.iter_mut().enumerate() {
            if !instance.finished {
                instance.finished = true;
                instance.eof_reached_ms = Some(eof_ms);
                self.decode_backpressure.on_finished(instance_index);
            }
        }
    }

    /// True when each instance in the logical track has samples or is finished.
    #[cfg(any(test, feature = "debug"))]
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
    #[cfg(any(test, feature = "debug"))]
    pub(crate) fn instance_buffer_fills(&self) -> Vec<(usize, usize)> {
        self.instances
            .iter()
            .map(|instance| (instance.meta.instance_id, instance.buffer.len()))
            .collect()
    }

    /// Fill-state aggregate for one logical track.
    #[cfg(any(test, feature = "debug"))]
    pub(crate) fn tracks_fill_state(&self) -> Vec<FillState> {
        let tracks: Vec<FillState> = self
            .track_instances
            .iter()
            .map(|instance_ids| {
                aggregate_fill_state(
                    instance_ids
                        .iter()
                        .map(|instance_index| self.instances[*instance_index].full),
                )
            })
            .collect();

        tracks
    }

    /// Fill-state aggregate for one logical track.
    #[cfg(any(test, feature = "debug"))]
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
    #[cfg(any(test, feature = "debug"))]
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
                let instance = &mut self.instances[*instance_index];
                let available = if !instance_needs_data(
                    instance,
                    self.consumed_samples,
                    self.sample_rate,
                    self.channels,
                ) {
                    usize::MAX
                } else if instance_fully_past_window(
                    instance,
                    self.consumed_samples,
                    self.sample_rate,
                    self.channels,
                ) {
                    instance.finished = true;
                    self.decode_backpressure.on_finished(*instance_index);
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

        let min_ready_samples = ready_samples_per_track.into_iter().min().unwrap_or(0);

        if min_ready_samples == 0 || min_ready_samples == usize::MAX {
            return None;
        }

        let to_consume = min_ready_samples.min(self.mix_chunk_samples);

        if !self.mix_finished()
            && to_consume < self.mix_chunk_samples
            && !self.decode_backpressure.has_waiters()
        {
            return None;
        }

        let mut logical_tracks = Vec::with_capacity(self.track_instances.len());
        for (track_index, track_indices) in self.track_instances.iter().enumerate() {
            let mut track_buffer = vec![0.0_f32; to_consume];

            #[cfg(feature = "buffer-map")]
            log_buffer_header(
                track_index,
                self.sample_rate,
                self.channels,
                self.consumed_samples,
            );

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

                #[cfg(feature = "buffer-map")]
                let divisor = 176;
                #[cfg(feature = "buffer-map")]
                let mut logging_buffer: Vec<&str> =
                    Vec::with_capacity((to_consume as f64 / divisor as f64).ceil() as usize);

                #[cfg(feature = "buffer-map")]
                let mut count = 1;
                #[cfg(feature = "buffer-map")]
                let mut aggregate_value = 0.0;

                let mut popped_samples = 0usize;
                for sample in track_buffer.iter_mut().take(to_consume) {
                    #[cfg(feature = "buffer-map")]
                    {
                        count += 1;
                    }

                    if let Some(value) = instance.buffer.pop_front() {
                        popped_samples = popped_samples.saturating_add(1);

                        #[cfg(feature = "buffer-map")]
                        if count % divisor == 0 {
                            logging_buffer.push(if aggregate_value != 0.0 { "X" } else { "_" });
                        }

                        #[cfg(feature = "buffer-map")]
                        {
                            aggregate_value += value;
                        }

                        *sample += value;
                    }
                }
                self.decode_backpressure
                    .on_samples_popped(*instance_index, popped_samples);
                debug!("Popped {} samples from i{}", popped_samples, instance_index);

                if popped_samples == 0 && !self.pop_warning.contains(&instance.meta.instance_id) {
                    warn!(
                        "ZERO! i{} ( finished: {}, ts: {}, total_samples: {} )",
                        instance.meta.instance_id,
                        instance.finished,
                        samples_to_ms(self.consumed_samples, self.sample_rate, self.channels),
                        instance.produced_samples + instance.zero_filled_samples
                    );
                    self.pop_warning.push(instance.meta.instance_id);
                }

                #[cfg(feature = "buffer-map")]
                log_buffer(instance, logging_buffer);
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
    #[cfg(any(test, feature = "debug"))]
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

    /// Shared backpressure handle used by decode workers to block until source buffers have room.
    pub(crate) fn decode_backpressure(&self) -> Arc<DecodeBackpressure> {
        Arc::clone(&self.decode_backpressure)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::container::prot::{
        ActiveWindow, RuntimeInstanceMeta, RuntimeInstancePlan, ShuffleSource,
    };

    use super::{BufferMixer, FillState, SourceKey};

    /// Build a small two-track runtime plan used by buffer mixer unit tests.
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
    /// Verifies packet routing writes samples only to matching source instances.
    fn route_packet_targets_and_zero_fills_instances() {
        let mut mixer = BufferMixer::new(simple_plan(), 48_000, 2, 8, HashMap::new(), 4);

        let decision = mixer.route_packet(&[1.0, 1.0, 0.5, 0.5], SourceKey::TrackId(1), 0.0);
        assert_eq!(decision.sample_targets_written, vec![0]);
        assert!(decision.zero_fill_targets_written.is_empty());
        assert!(!decision.ignored);
    }

    #[test]
    /// Verifies mix readiness and sample consumption stay in lockstep.
    fn readiness_and_take_samples_are_synchronized() {
        let mut mixer = BufferMixer::new(simple_plan(), 48_000, 2, 16, HashMap::new(), 4);

        mixer.route_packet(&[1.0, 1.0, 1.0, 1.0], SourceKey::TrackId(1), 0.0);
        assert!(!mixer.mix_ready());
        assert!(mixer.take_samples().is_none());

        mixer.route_packet(&[0.5, 0.5, 0.5, 0.5], SourceKey::TrackId(2), 0.0);
        assert!(mixer.mix_ready());

        let mixed = mixer.take_samples().expect("mixed samples");
        assert_eq!(mixed.len(), 4);
        assert_eq!(mixed, vec![0.75, 0.75, 0.75, 0.75]);
    }

    #[test]
    /// Verifies finish signals propagate to per-track and global finished state.
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
    /// Verifies aggregate fill-state reporting reflects per-instance fullness.
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

    #[test]
    /// Verifies packets before a window start are represented as aligned zero-fill.
    fn route_packet_zero_fills_when_packet_is_before_window_start() {
        let plan = RuntimeInstancePlan {
            logical_track_count: 1,
            instances: vec![RuntimeInstanceMeta {
                instance_id: 0,
                logical_track_index: 0,
                slot_index: 0,
                source_key: ShuffleSource::TrackId(1),
                active_windows: vec![ActiveWindow {
                    start_ms: 1000,
                    end_ms: Some(2000),
                }],
                selection_index: 0,
                occurrence_index: 0,
            }],
            event_boundaries_ms: vec![0, 1000],
        };
        let mut mixer = BufferMixer::new(plan, 48_000, 2, 16, HashMap::new(), 4);

        let decision = mixer.route_packet(&[1.0, 1.0, 1.0, 1.0], SourceKey::TrackId(1), 0.0);
        assert!(decision.sample_targets_written.is_empty());
        assert_eq!(decision.zero_fill_targets_written, vec![0]);
        assert!(mixer.mix_ready());

        let mixed = mixer.take_samples().expect("zero-filled samples");
        assert_eq!(mixed, vec![0.0, 0.0, 0.0, 0.0]);
    }
}
