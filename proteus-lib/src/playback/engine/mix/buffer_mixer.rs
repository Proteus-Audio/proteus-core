//! Schedule-driven source router and logical-track mixer.

use log::{debug, error, info, warn};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex};

use crate::logging::log_on_line;
use crate::{
    container::prot::{RuntimeInstanceMeta, RuntimeInstancePlan, ShuffleSource},
    logging::{clear_logfile, log},
    playback::engine::mix::utils::{map_cover, Cover},
    track,
};

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
    buffer: VecDeque<f32>,
    buffer_capacity_samples: usize,
    full: bool,
    finished: bool,
    produced_samples: u64,
    zero_filled_samples: u64,
    eof_reached_ms: Option<u64>,
}

#[derive(Debug, Clone)]
struct DecodeBackpressureInstance {
    capacity_samples: usize,
    buffered_samples: usize,
    finished: bool,
}

#[derive(Debug, Default)]
struct DecodeBackpressureState {
    shutdown: bool,
    waiting_threads: usize,
    instances: Vec<DecodeBackpressureInstance>,
    source_to_instances: HashMap<SourceKey, Vec<usize>>,
}

#[derive(Debug, Default)]
pub(crate) struct DecodeBackpressure {
    state: Mutex<DecodeBackpressureState>,
    cv: Condvar,
}

impl DecodeBackpressure {
    fn from_instances(instances: &[BufferInstance]) -> Self {
        let mut state = DecodeBackpressureState::default();
        state.instances.reserve(instances.len());
        for (index, instance) in instances.iter().enumerate() {
            let source = SourceKey::from(&instance.meta.source_key);
            state
                .source_to_instances
                .entry(source.clone())
                .or_default()
                .push(index);
            state.instances.push(DecodeBackpressureInstance {
                capacity_samples: instance.buffer_capacity_samples,
                buffered_samples: instance.buffer.len(),
                finished: instance.finished,
            });
        }
        Self {
            state: Mutex::new(state),
            cv: Condvar::new(),
        }
    }

    pub(crate) fn wait_for_source_room(
        &self,
        source: &SourceKey,
        required_samples: usize,
        abort: &std::sync::atomic::AtomicBool,
    ) -> bool {
        if required_samples == 0 {
            return true;
        }

        let mut guard = self.state.lock().unwrap();
        let mut wait_count = 0usize;
        loop {
            if guard.shutdown || abort.load(std::sync::atomic::Ordering::Relaxed) {
                debug!(
                    "decode_backpressure wait abort/shutdown: source={:?} required_samples={} shutdown={} abort={}",
                    source,
                    required_samples,
                    guard.shutdown,
                    abort.load(std::sync::atomic::Ordering::Relaxed)
                );
                return false;
            }
            let status = source_room_status(&guard, source, required_samples);
            if status.allowed {
                if wait_count > 0 {
                    debug!(
                        "decode_backpressure wait satisfied: source={:?} required_samples={} waits={} details={}",
                        source,
                        required_samples,
                        wait_count,
                        status.summary
                    );
                }
                return true;
            }
            if wait_count == 0 {
                debug!(
                    "decode_backpressure wait start: source={:?} required_samples={} details={}",
                    source, required_samples, status.summary
                );
            } else {
                debug!(
                    "decode_backpressure wait continue: source={:?} required_samples={} waits={} details={}",
                    source,
                    required_samples,
                    wait_count,
                    status.summary
                );
            }
            wait_count = wait_count.saturating_add(1);
            guard.waiting_threads = guard.waiting_threads.saturating_add(1);
            guard = self.cv.wait(guard).unwrap();
            guard.waiting_threads = guard.waiting_threads.saturating_sub(1);
            debug!(
                "decode_backpressure wait wake: source={:?} required_samples={} waits={}",
                source, required_samples, wait_count
            );
        }
    }

    fn on_samples_pushed(&self, instance_index: usize, pushed_samples: usize, is_full: bool) {
        if pushed_samples == 0 && !is_full {
            return;
        }
        let mut guard = self.state.lock().unwrap();
        if let Some(instance) = guard.instances.get_mut(instance_index) {
            instance.buffered_samples = instance
                .buffered_samples
                .saturating_add(pushed_samples)
                .min(instance.capacity_samples.max(1));
            debug!(
                "decode_backpressure push: instance_index={} pushed_samples={} buffered={} capacity={} finished={} full_flag={}",
                instance_index,
                pushed_samples,
                instance.buffered_samples,
                instance.capacity_samples,
                instance.finished,
                is_full
            );
        }
        if pushed_samples > 0 || !is_full {
            self.cv.notify_all();
        }
    }

    fn on_samples_popped(&self, instance_index: usize, popped_samples: usize) {
        if popped_samples == 0 {
            return;
        }
        let mut guard = self.state.lock().unwrap();
        if let Some(instance) = guard.instances.get_mut(instance_index) {
            instance.buffered_samples = instance.buffered_samples.saturating_sub(popped_samples);
            debug!(
                "decode_backpressure pop: instance_index={} popped_samples={} buffered={} capacity={} finished={}",
                instance_index,
                popped_samples,
                instance.buffered_samples,
                instance.capacity_samples,
                instance.finished
            );
        }
        self.cv.notify_all();
    }

    fn on_finished(&self, instance_index: usize) {
        let mut guard = self.state.lock().unwrap();
        if let Some(instance) = guard.instances.get_mut(instance_index) {
            if !instance.finished {
                instance.finished = true;
                debug!(
                    "decode_backpressure finished: instance_index={} buffered={} capacity={}",
                    instance_index, instance.buffered_samples, instance.capacity_samples
                );
                self.cv.notify_all();
            }
        }
    }

    pub(crate) fn shutdown(&self) {
        let mut guard = self.state.lock().unwrap();
        guard.shutdown = true;
        debug!("decode_backpressure shutdown");
        self.cv.notify_all();
    }

    pub(crate) fn has_waiters(&self) -> bool {
        self.state.lock().unwrap().waiting_threads > 0
    }
}

#[derive(Debug, Clone)]
struct SourceRoomStatus {
    allowed: bool,
    summary: String,
}

fn source_room_status(
    state: &DecodeBackpressureState,
    source: &SourceKey,
    required_samples: usize,
) -> SourceRoomStatus {
    let Some(instance_indices) = state.source_to_instances.get(source) else {
        return SourceRoomStatus {
            allowed: true,
            summary: "no_instances".to_string(),
        };
    };

    let mut saw_unfinished = false;
    let mut all_have_target_room = true;
    let mut any_has_progress_room = false;
    let mut parts = Vec::new();

    for instance_index in instance_indices {
        let Some(instance) = state.instances.get(*instance_index) else {
            continue;
        };
        if instance.finished {
            parts.push(format!("i{}:finished", instance_index));
            continue;
        }
        saw_unfinished = true;
        let free = instance
            .capacity_samples
            .saturating_sub(instance.buffered_samples);

        // A packet may be larger than the per-instance ring capacity, so "full packet room"
        // is impossible in that case. Clamp the target to preserve liveness.
        let target_room = required_samples.min(instance.capacity_samples.max(1));
        all_have_target_room &= free >= target_room;
        any_has_progress_room |= free > 0;
        parts.push(format!(
            "i{}:buf={}/{} free={} target={}",
            instance_index, instance.buffered_samples, instance.capacity_samples, free, target_room
        ));
    }

    if !saw_unfinished {
        return SourceRoomStatus {
            allowed: true,
            summary: format!("all_finished [{}]", parts.join(", ")),
        };
    }

    // Prefer the stricter "all instances have room" check, but allow progress when at least one
    // unfinished instance can accept data. This avoids deadlocking decode on future/repeated
    // instances that are already full while current playback still needs packets from the source.
    let allowed = all_have_target_room || any_has_progress_room;
    SourceRoomStatus {
        allowed,
        summary: format!(
            "allowed={} all_target={} any_progress={} [{}]",
            allowed,
            all_have_target_room,
            any_has_progress_room,
            parts.join(", ")
        ),
    }
}

impl BufferInstance {
    fn new(meta: RuntimeInstanceMeta, capacity_samples: usize) -> Self {
        Self {
            meta,
            buffer: VecDeque::with_capacity(capacity_samples.max(1)),
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
        clear_logfile();
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

            // let cover = map_cover(&overlap, samples.len(), Some(80));
            let cover = map_cover(&overlap, samples.len(), None);

            let first_window = &instance.meta.active_windows.first();
            let first_window_start = if first_window.is_some() && first_window.unwrap().start_ms > 0
            {
                first_window.unwrap().start_ms as f64 / 1000.0
            } else {
                0.0
            };

            let last_of_window = overlap
                .last()
                .map(|(_, end)| *end < samples.len())
                .unwrap_or(false);

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
                            push.written_samples,
                            instance.full,
                        );
                        if push.wrote_any {
                            wrote_real = true;
                        }
                        instance.produced_samples = instance
                            .produced_samples
                            .saturating_add((end_sample - start_sample) as u64);
                    }
                    Cover::Underlay((start_sample, end_sample)) => {
                        let length = end_sample - start_sample;

                        let push = push_slice(
                            &mut instance.buffer,
                            instance.buffer_capacity_samples,
                            &vec![0.0; length],
                            &mut instance.full,
                        );
                        self.decode_backpressure.on_samples_pushed(
                            instance_index,
                            push.written_samples,
                            instance.full,
                        );
                        if push.wrote_any {
                            wrote_zero = true;
                        }
                        instance.zero_filled_samples = instance
                            .zero_filled_samples
                            .saturating_add((end_sample - start_sample) as u64);
                    }
                    _ => {}
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
    pub(crate) fn instance_buffer_fills(&self) -> Vec<(usize, usize)> {
        self.instances
            .iter()
            .map(|instance| (instance.meta.instance_id, instance.buffer.len()))
            .collect()
    }

    /// Fill-state aggregate for one logical track.
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
                let instance = &mut self.instances[*instance_index];
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

                let divisor = 176;
                let mut logging_buffer: Vec<&str> =
                    Vec::with_capacity((to_consume as f64 / divisor as f64).ceil() as usize);
                let mut count = 1;
                let mut aggregate_value = 0.0;
                let mut popped_samples = 0usize;
                for sample in track_buffer.iter_mut().take(to_consume) {
                    count += 1;
                    if let Some(value) = instance.buffer.pop_front() {
                        popped_samples = popped_samples.saturating_add(1);
                        if count % divisor == 0 {
                            logging_buffer.push(if aggregate_value != 0.0 { "X" } else { "_" });
                        }
                        aggregate_value += value;
                        *sample += value;
                    }
                }
                self.decode_backpressure
                    .on_samples_popped(*instance_index, popped_samples);
                debug!("Popped {} samples from i{}", popped_samples, instance_index);

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

#[derive(Debug, Clone, Copy, Default)]
struct PushResult {
    written_samples: usize,
    wrote_any: bool,
}

fn push_slice(
    buffer: &mut VecDeque<f32>,
    capacity_samples: usize,
    slice: &[f32],
    full_flag: &mut bool,
) -> PushResult {
    let mut result = PushResult::default();
    let mut overflow = false;
    for sample in slice.iter().copied() {
        if buffer.len() >= capacity_samples.max(1) {
            overflow = true;
            break;
        }
        buffer.push_back(sample);
        result.wrote_any = true;
        result.written_samples += 1;
    }
    *full_flag = overflow;
    result
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
    true
    // let start_sample = window_start_samples(instance, sample_rate, channels);
    // let end_sample = window_end_samples(instance, sample_rate, channels);
    // consumed_samples >= start_sample && end_sample.map(|end| consumed_samples < end).unwrap_or(true)
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

fn instance_past_window_ts(instance: &BufferInstance, ts: &f64) -> bool {
    let end: Option<f64> = instance
        .meta
        .active_windows
        .last()
        .and_then(|window| window.end_ms.map(|end| end as f64 / 1000.0));
    let Some(end_ts) = end else {
        return false;
    };

    *ts >= end_ts
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

fn log_buffer_header(
    logical_track_index: usize,
    sample_rate: u32,
    channels: usize,
    consumed_samples: usize,
) {
    let consumed_ms = samples_to_ms(consumed_samples, sample_rate, channels);

    log(&format!("T{:?}\n{}\n", logical_track_index, consumed_ms));
}

fn log_buffer(instance: &BufferInstance, map: Vec<&str>) {
    let instance_id = instance.meta.instance_id;
    log(&format!("[{}] <- i{}\n", map.join(""), instance_id));

    // let result = log_on_line(&format!("[{}]", map.join("")), instance_id + 2);
    // if let Err(e) = result {
    //     error!("Failed to log_on_line: {}", e);
    // }
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
        assert!(decision.zero_fill_targets_written.is_empty());
        assert!(!decision.ignored);
    }

    #[test]
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

    #[test]
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
