//! Runtime instance plan construction and mix-settings delegation.

use std::collections::HashMap;

use crate::container::play_settings::PlaySettingsFile;

use super::helpers::*;
use super::schedule::seconds_to_ms;
use super::types::{RuntimeInstancePlan, ShuffleScheduleEntry, ShuffleSource};
use super::{versioned_tracks, Prot, ProtSource};

impl Prot {
    /// Return the full timestamped shuffle schedule grouped by logical track.
    ///
    /// Each entry is `(time_seconds, track_groups)`, where `track_groups`
    /// contains one element per logical track and each inner vector contains all
    /// selected source IDs/paths for that track (based on `selections_count`).
    pub fn get_shuffle_schedule(&self) -> Vec<(f64, Vec<Vec<String>>)> {
        let slot_spans = self.logical_track_slot_spans();
        if self.shuffle_schedule.is_empty() {
            let current = self.get_ids();
            if current.is_empty() {
                return Vec::new();
            }
            return vec![(0.0, group_ids_by_slot_spans(&current, &slot_spans))];
        }

        self.shuffle_schedule
            .iter()
            .map(|entry| {
                let ids: Vec<String> = entry
                    .sources
                    .iter()
                    .map(|source| match source {
                        ShuffleSource::TrackId(track_id) => track_id.to_string(),
                        ShuffleSource::FilePath(path) => path.clone(),
                    })
                    .collect();
                (
                    entry.at_ms as f64 / 1000.0,
                    group_ids_by_slot_spans(&ids, &slot_spans),
                )
            })
            .collect()
    }

    /// Expand grouped shuffle schedule entries into concrete source instances.
    ///
    /// The resulting plan preserves duplicates as unique instances and clips all
    /// windows to `start_time`.
    pub(crate) fn build_runtime_instance_plan(&self, start_time: f64) -> RuntimeInstancePlan {
        let start_ms = seconds_to_ms(start_time);
        let mut schedule = runtime_schedule(self);
        if schedule.is_empty() {
            return RuntimeInstancePlan {
                logical_track_count: 0,
                instances: Vec::new(),
                event_boundaries_ms: Vec::new(),
            };
        }

        let slot_count = schedule
            .iter()
            .map(|entry| entry.sources.len())
            .max()
            .unwrap_or(0);
        let (slot_layout, logical_track_count) =
            build_slot_layout(slot_count, &self.logical_track_slot_spans());
        normalize_schedule_sources(&mut schedule, slot_count);
        let event_boundaries_ms = collect_event_boundaries(&schedule, start_ms);
        let instances = collect_runtime_instances(&schedule, &slot_layout, slot_count, start_ms);

        RuntimeInstancePlan {
            logical_track_count,
            instances,
            event_boundaries_ms,
        }
    }

    /// Return per-track `(level, pan)` settings keyed by track key.
    pub fn get_track_mix_settings(&self) -> HashMap<u16, (f32, f32)> {
        let mut settings = HashMap::new();

        if let ProtSource::Paths { file_paths, .. } = &self.source {
            let mut slot_index: u16 = 0;
            for track in file_paths {
                let selections = track.selections_count.max(1);
                for _ in 0..selections {
                    settings.insert(slot_index, (track.level, track.pan));
                    slot_index = slot_index.saturating_add(1);
                }
            }
            return settings;
        }

        let tracks = self.play_settings.as_ref().and_then(versioned_tracks);

        if let Some(tracks) = tracks {
            let mut slot_index: u16 = 0;
            for track in tracks {
                let selections = track.selections_count.max(1);
                for _ in 0..selections {
                    settings.insert(slot_index, (track.level, track.pan));
                    slot_index = slot_index.saturating_add(1);
                }
            }
        }

        settings
    }

    /// Update the `(level, pan)` mix settings for a selected slot.
    ///
    /// Returns `true` when a matching slot was updated.
    pub fn set_slot_mix_settings(&mut self, slot_index: usize, level: f32, pan: f32) -> bool {
        let level = sanitize_level(level);
        let pan = sanitize_pan(pan);

        if let ProtSource::Paths { file_paths, .. } = &mut self.source {
            if let Some(track) = get_paths_track_for_slot_mut(file_paths, slot_index) {
                track.level = level;
                track.pan = pan;
                return true;
            }
            return false;
        }

        self.play_settings
            .as_mut()
            .and_then(super::versioned_tracks_mut)
            .map(|tracks| update_settings_track_slot(tracks.as_mut_slice(), slot_index, level, pan))
            .unwrap_or(false)
    }

    /// Return all slot indices that share the same track settings as `slot_index`.
    pub fn linked_slot_indices(&self, slot_index: usize) -> Option<Vec<usize>> {
        if let ProtSource::Paths { file_paths, .. } = &self.source {
            return linked_paths_slots(file_paths, slot_index);
        }

        let tracks = self.play_settings.as_ref().and_then(versioned_tracks)?;

        linked_settings_slots(tracks, slot_index)
    }

    /// Return the number of possible unique selections based on track settings.
    pub fn count_possible_combinations(&self) -> Option<u128> {
        if let ProtSource::Paths { file_paths, .. } = &self.source {
            return count_paths_track_combinations(file_paths);
        }

        let play_settings = self.play_settings.as_ref()?;
        match play_settings {
            PlaySettingsFile::Legacy(file) => {
                count_legacy_track_combinations(file.settings.inner())
            }
            PlaySettingsFile::V1(_) | PlaySettingsFile::V2(_) | PlaySettingsFile::V3(_) => {
                count_settings_track_combinations(versioned_tracks(play_settings).unwrap_or(&[]))
            }
            PlaySettingsFile::Unknown { .. } => None,
        }
    }

    pub(crate) fn logical_track_slot_spans(&self) -> Vec<usize> {
        if let ProtSource::Paths { file_paths, .. } = &self.source {
            return file_paths
                .iter()
                .filter_map(|track| {
                    if track.file_paths.is_empty() {
                        return None;
                    }
                    let span = track.selections_count as usize;
                    if span == 0 {
                        None
                    } else {
                        Some(span)
                    }
                })
                .collect();
        }

        match self.play_settings.as_ref() {
            Some(play_settings) if versioned_tracks(play_settings).is_some() => {
                versioned_tracks(play_settings)
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|track| {
                        if track.ids.is_empty() {
                            return None;
                        }
                        let span = track.selections_count as usize;
                        if span == 0 {
                            None
                        } else {
                            Some(span)
                        }
                    })
                    .collect()
            }
            Some(PlaySettingsFile::Legacy(file)) => file
                .settings
                .inner()
                .tracks
                .iter()
                .filter(|track| track.starting_index.is_some() && track.length.is_some())
                .map(|_| 1usize)
                .collect(),
            _ => Vec::new(),
        }
    }
}

fn runtime_schedule(prot: &Prot) -> Vec<ShuffleScheduleEntry> {
    if !prot.shuffle_schedule.is_empty() {
        return prot.shuffle_schedule.clone();
    }

    let fallback_sources: Vec<ShuffleSource> = if let Some(track_ids) = &prot.track_ids {
        track_ids
            .iter()
            .copied()
            .map(ShuffleSource::TrackId)
            .collect()
    } else if let Some(track_paths) = &prot.track_paths {
        track_paths
            .iter()
            .cloned()
            .map(ShuffleSource::FilePath)
            .collect()
    } else {
        Vec::new()
    };

    if fallback_sources.is_empty() {
        Vec::new()
    } else {
        vec![ShuffleScheduleEntry {
            at_ms: 0,
            sources: fallback_sources,
        }]
    }
}

fn normalize_schedule_sources(schedule: &mut [ShuffleScheduleEntry], slot_count: usize) {
    let mut last_sources = vec![None::<ShuffleSource>; slot_count];
    for entry in schedule {
        for (slot_index, last_source) in last_sources.iter_mut().enumerate().take(slot_count) {
            if slot_index < entry.sources.len() {
                *last_source = Some(entry.sources[slot_index].clone());
            } else if let Some(previous) = last_source.clone() {
                entry.sources.push(previous);
            }
        }
    }
}

fn collect_event_boundaries(schedule: &[ShuffleScheduleEntry], start_ms: u64) -> Vec<u64> {
    schedule
        .iter()
        .filter_map(|entry| (entry.at_ms >= start_ms).then(|| entry.at_ms - start_ms))
        .collect()
}

fn collect_runtime_instances(
    schedule: &[ShuffleScheduleEntry],
    slot_layout: &[(usize, usize)],
    slot_count: usize,
    start_ms: u64,
) -> Vec<super::types::RuntimeInstanceMeta> {
    let mut instances = Vec::new();
    let mut next_instance_id = 0usize;
    let mut occurrence_counters: HashMap<(usize, usize), usize> = HashMap::new();

    for slot_index in 0..slot_count {
        let (logical_track_index, selection_index) = slot_layout
            .get(slot_index)
            .copied()
            .unwrap_or((slot_index, 0));
        next_instance_id = collect_slot_instances(
            schedule,
            slot_index,
            logical_track_index,
            selection_index,
            start_ms,
            next_instance_id,
            &mut occurrence_counters,
            &mut instances,
        );
    }

    instances
}

#[allow(clippy::too_many_arguments)]
fn collect_slot_instances(
    schedule: &[ShuffleScheduleEntry],
    slot_index: usize,
    logical_track_index: usize,
    selection_index: usize,
    start_ms: u64,
    mut next_instance_id: usize,
    occurrence_counters: &mut HashMap<(usize, usize), usize>,
    instances: &mut Vec<super::types::RuntimeInstanceMeta>,
) -> usize {
    let mut current_source: Option<ShuffleSource> = None;
    let mut current_start = 0_u64;

    for entry in schedule {
        let Some(source) = entry.sources.get(slot_index).cloned() else {
            continue;
        };

        match current_source.as_ref() {
            None => {
                current_source = Some(source);
                current_start = entry.at_ms;
            }
            Some(active_source) if active_source == &source => {}
            Some(active_source) => {
                next_instance_id = push_runtime_instance(
                    next_instance_id,
                    logical_track_index,
                    slot_index,
                    selection_index,
                    active_source,
                    current_start,
                    Some(entry.at_ms),
                    start_ms,
                    occurrence_counters,
                    instances,
                );
                current_source = Some(source);
                current_start = entry.at_ms;
            }
        }
    }

    if let Some(source) = current_source.as_ref() {
        next_instance_id = push_runtime_instance(
            next_instance_id,
            logical_track_index,
            slot_index,
            selection_index,
            source,
            current_start,
            None,
            start_ms,
            occurrence_counters,
            instances,
        );
    }

    next_instance_id
}

#[allow(clippy::too_many_arguments)]
fn push_runtime_instance(
    next_instance_id: usize,
    logical_track_index: usize,
    slot_index: usize,
    selection_index: usize,
    source: &ShuffleSource,
    segment_start_ms: u64,
    segment_end_ms: Option<u64>,
    start_ms: u64,
    occurrence_counters: &mut HashMap<(usize, usize), usize>,
    instances: &mut Vec<super::types::RuntimeInstanceMeta>,
) -> usize {
    if let Some(meta) = build_segment_instance(
        next_instance_id,
        logical_track_index,
        slot_index,
        selection_index,
        source,
        segment_start_ms,
        segment_end_ms,
        start_ms,
        occurrence_counters,
    ) {
        instances.push(meta);
        next_instance_id + 1
    } else {
        next_instance_id
    }
}

#[cfg(test)]
mod tests;
