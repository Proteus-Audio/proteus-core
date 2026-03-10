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
        let fallback_sources: Vec<ShuffleSource> = if let Some(track_ids) = &self.track_ids {
            track_ids
                .iter()
                .copied()
                .map(ShuffleSource::TrackId)
                .collect()
        } else if let Some(track_paths) = &self.track_paths {
            track_paths
                .iter()
                .cloned()
                .map(ShuffleSource::FilePath)
                .collect()
        } else {
            Vec::new()
        };

        let mut schedule: Vec<ShuffleScheduleEntry> = if self.shuffle_schedule.is_empty() {
            if fallback_sources.is_empty() {
                Vec::new()
            } else {
                vec![ShuffleScheduleEntry {
                    at_ms: 0,
                    sources: fallback_sources,
                }]
            }
        } else {
            self.shuffle_schedule.clone()
        };

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

        // Keep schedule entries rectangular by carrying previous values for
        // missing slots.
        let mut last_sources = vec![None::<ShuffleSource>; slot_count];
        for entry in &mut schedule {
            for (slot_index, last_source) in last_sources.iter_mut().enumerate().take(slot_count) {
                if slot_index < entry.sources.len() {
                    *last_source = Some(entry.sources[slot_index].clone());
                } else if let Some(previous) = last_source.clone() {
                    entry.sources.push(previous);
                }
            }
        }

        let event_boundaries_ms = schedule
            .iter()
            .filter_map(|entry| {
                if entry.at_ms >= start_ms {
                    Some(entry.at_ms - start_ms)
                } else {
                    None
                }
            })
            .collect();

        let mut instances = Vec::new();
        let mut next_instance_id = 0usize;
        let mut occurrence_counters: HashMap<(usize, usize), usize> = HashMap::new();

        for slot_index in 0..slot_count {
            let (logical_track_index, selection_index) = slot_layout
                .get(slot_index)
                .copied()
                .unwrap_or((slot_index, 0));

            let mut current_source: Option<ShuffleSource> = None;
            let mut current_start = 0_u64;

            for (event_index, entry) in schedule.iter().enumerate() {
                let source = entry.sources.get(slot_index).cloned();
                if source.is_none() {
                    continue;
                }
                let source = source.unwrap();

                if current_source.is_none() {
                    current_source = Some(source);
                    current_start = entry.at_ms;
                    continue;
                }

                if current_source.as_ref() == Some(&source) {
                    continue;
                }

                let segment_end = entry.at_ms;
                if let Some(meta) = build_segment_instance(
                    next_instance_id,
                    logical_track_index,
                    slot_index,
                    selection_index,
                    current_source.as_ref().unwrap(),
                    current_start,
                    Some(segment_end),
                    start_ms,
                    &mut occurrence_counters,
                ) {
                    instances.push(meta);
                    next_instance_id += 1;
                }

                current_source = Some(source);
                current_start = entry.at_ms;

                if event_index + 1 == schedule.len() {
                    break;
                }
            }

            if let Some(source) = current_source.as_ref() {
                if let Some(meta) = build_segment_instance(
                    next_instance_id,
                    logical_track_index,
                    slot_index,
                    selection_index,
                    source,
                    current_start,
                    None,
                    start_ms,
                    &mut occurrence_counters,
                ) {
                    instances.push(meta);
                    next_instance_id += 1;
                }
            }
        }

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

#[cfg(test)]
mod tests {
    use super::super::types::PathsTrack;
    use super::*;
    use crate::container::info::Info;
    use crate::container::play_settings::SettingsTrack;

    fn test_info() -> Info {
        Info {
            file_paths: Vec::new(),
            duration_map: HashMap::new(),
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
        }
    }

    fn prot_from_paths(file_paths: Vec<PathsTrack>, dictionary: Vec<String>) -> Prot {
        Prot {
            info: test_info(),
            source: ProtSource::Paths {
                file_paths,
                file_paths_dictionary: dictionary,
            },
            track_ids: None,
            track_paths: None,
            duration: 0.0,
            shuffle_schedule: Vec::new(),
            play_settings: None,
            impulse_response_spec: None,
            impulse_response_tail_db: None,
            effects: None,
        }
    }

    fn prot_from_container(file_path: &str) -> Prot {
        Prot {
            info: test_info(),
            source: ProtSource::Container {
                file_path: file_path.to_string(),
            },
            track_ids: None,
            track_paths: None,
            duration: 0.0,
            shuffle_schedule: Vec::new(),
            play_settings: None,
            impulse_response_spec: None,
            impulse_response_tail_db: None,
            effects: None,
        }
    }

    #[test]
    fn get_track_mix_settings_repeats_by_selections_count_for_paths_tracks() {
        let prot = prot_from_paths(
            vec![PathsTrack {
                file_paths: vec!["a.wav".to_string()],
                level: 0.7,
                pan: -0.3,
                selections_count: 2,
                shuffle_points: vec![],
            }],
            vec!["a.wav".to_string()],
        );

        let settings = prot.get_track_mix_settings();
        assert_eq!(settings.get(&0), Some(&(0.7, -0.3)));
        assert_eq!(settings.get(&1), Some(&(0.7, -0.3)));
    }

    #[test]
    fn set_slot_mix_settings_updates_paths_track() {
        let mut prot = prot_from_paths(
            vec![PathsTrack {
                file_paths: vec!["a.wav".to_string()],
                level: 1.0,
                pan: 0.0,
                selections_count: 2,
                shuffle_points: vec![],
            }],
            vec!["a.wav".to_string()],
        );

        assert!(prot.set_slot_mix_settings(1, 0.4, 0.6));
        let settings = prot.get_track_mix_settings();
        assert_eq!(settings.get(&0), Some(&(0.4, 0.6)));
        assert_eq!(settings.get(&1), Some(&(0.4, 0.6)));
    }

    #[test]
    fn get_track_mix_settings_includes_v3_tracks() {
        use crate::container::play_settings::{
            PlaySettingsContainer, PlaySettingsV3, PlaySettingsV3File,
        };

        let play_settings = PlaySettingsFile::V3(PlaySettingsV3File {
            settings: PlaySettingsContainer::Flat(PlaySettingsV3 {
                effects: Vec::new(),
                tracks: vec![SettingsTrack {
                    level: 0.25,
                    pan: 0.2,
                    ids: vec![1],
                    name: "Track".to_string(),
                    safe_name: "track".to_string(),
                    selections_count: 2,
                    shuffle_points: vec![],
                }],
            }),
        });

        let prot = Prot {
            info: test_info(),
            source: ProtSource::Container {
                file_path: "dummy.prot".to_string(),
            },
            track_ids: Some(vec![1, 1]),
            track_paths: None,
            duration: 0.0,
            shuffle_schedule: Vec::new(),
            play_settings: Some(play_settings),
            impulse_response_spec: None,
            impulse_response_tail_db: None,
            effects: None,
        };

        let settings = prot.get_track_mix_settings();
        assert_eq!(settings.get(&0), Some(&(0.25, 0.2)));
        assert_eq!(settings.get(&1), Some(&(0.25, 0.2)));
    }

    #[test]
    fn linked_slot_indices_returns_all_slots_for_same_track() {
        let prot = prot_from_paths(
            vec![
                PathsTrack {
                    file_paths: vec!["a.wav".to_string()],
                    level: 1.0,
                    pan: 0.0,
                    selections_count: 2,
                    shuffle_points: vec![],
                },
                PathsTrack {
                    file_paths: vec!["b.wav".to_string()],
                    level: 1.0,
                    pan: 0.0,
                    selections_count: 1,
                    shuffle_points: vec![],
                },
            ],
            vec!["a.wav".to_string(), "b.wav".to_string()],
        );

        assert_eq!(prot.linked_slot_indices(0), Some(vec![0, 1]));
        assert_eq!(prot.linked_slot_indices(1), Some(vec![0, 1]));
        assert_eq!(prot.linked_slot_indices(2), Some(vec![2]));
        assert_eq!(prot.linked_slot_indices(3), None);
    }

    #[test]
    fn get_shuffle_schedule_groups_by_paths_track_selections_count() {
        let mut prot = prot_from_paths(
            vec![
                PathsTrack {
                    file_paths: vec!["a.wav".to_string(), "b.wav".to_string()],
                    level: 1.0,
                    pan: 0.0,
                    selections_count: 2,
                    shuffle_points: vec![],
                },
                PathsTrack {
                    file_paths: vec!["c.wav".to_string()],
                    level: 1.0,
                    pan: 0.0,
                    selections_count: 1,
                    shuffle_points: vec![],
                },
            ],
            vec![
                "a.wav".to_string(),
                "b.wav".to_string(),
                "c.wav".to_string(),
            ],
        );
        prot.shuffle_schedule = vec![ShuffleScheduleEntry {
            at_ms: 0,
            sources: vec![
                ShuffleSource::FilePath("a.wav".to_string()),
                ShuffleSource::FilePath("b.wav".to_string()),
                ShuffleSource::FilePath("c.wav".to_string()),
            ],
        }];

        let schedule = prot.get_shuffle_schedule();
        assert_eq!(schedule.len(), 1);
        assert_eq!(
            schedule[0].1,
            vec![
                vec!["a.wav".to_string(), "b.wav".to_string()],
                vec!["c.wav".to_string()],
            ]
        );
    }

}
