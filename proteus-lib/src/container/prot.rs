//! Container model and play settings parsing for `.prot`/`.mka`.

use matroska::Matroska;
use rand::Rng;
use std::collections::{BTreeSet, HashMap, HashSet};

use log::{error, info, warn};

use crate::container::info::*;
use crate::container::play_settings::{PlaySettingsFile, PlaySettingsLegacy, SettingsTrack};
use crate::dsp::effects::convolution_reverb::{
    parse_impulse_response_spec, parse_impulse_response_tail_db, ImpulseResponseSpec,
};
use crate::dsp::effects::AudioEffect;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ShuffleSource {
    TrackId(u32),
    FilePath(String),
}

#[derive(Debug, Clone)]
pub(crate) struct ShuffleScheduleEntry {
    pub at_ms: u64,
    pub sources: Vec<ShuffleSource>,
}

/// Active time range for one instance in milliseconds relative to playback start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveWindow {
    pub start_ms: u64,
    pub end_ms: Option<u64>,
}

/// Runtime metadata for one concrete source instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeInstanceMeta {
    pub instance_id: usize,
    pub logical_track_index: usize,
    pub slot_index: usize,
    pub source_key: ShuffleSource,
    pub active_windows: Vec<ActiveWindow>,
    pub selection_index: usize,
    pub occurrence_index: usize,
}

/// Expanded runtime plan used by schedule-driven routing/mixing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeInstancePlan {
    pub logical_track_count: usize,
    pub instances: Vec<RuntimeInstanceMeta>,
    pub event_boundaries_ms: Vec<u64>,
}

/// Parsed `.prot` container with resolved tracks and playback metadata.
#[derive(Debug, Clone)]
pub struct Prot {
    pub info: Info,
    file_path: Option<String>,
    file_paths: Option<Vec<PathsTrack>>,
    file_paths_dictionary: Option<Vec<String>>,
    track_ids: Option<Vec<u32>>,
    track_paths: Option<Vec<String>>,
    duration: f64,
    shuffle_schedule: Vec<ShuffleScheduleEntry>,
    play_settings: Option<PlaySettingsFile>,
    impulse_response_spec: Option<ImpulseResponseSpec>,
    impulse_response_tail_db: Option<f32>,
    effects: Option<Vec<AudioEffect>>,
}

impl Prot {
    /// Load a single container file and resolve tracks.
    pub fn new(file_path: &str) -> Self {
        let info = Info::new(file_path.to_string());

        println!("Info: {:?}", info);

        let mut this = Self {
            info,
            file_path: Some(file_path.to_string()),
            file_paths: None,
            file_paths_dictionary: None,
            track_ids: None,
            track_paths: None,
            duration: 0.0,
            shuffle_schedule: Vec::new(),
            play_settings: None,
            impulse_response_spec: None,
            impulse_response_tail_db: None,
            effects: None,
        };

        this.load_play_settings();
        this.refresh_tracks();

        this
    }

    /// Build a container from multiple standalone file path sets.
    pub fn new_from_file_paths(file_paths: Vec<PathsTrack>) -> Self {
        let mut file_paths_dictionary = Vec::new();
        // Add all file paths to file_paths_dictionary
        // but do not add duplicates
        for track in file_paths.clone() {
            for path in &track.file_paths {
                if !file_paths_dictionary.contains(path) {
                    file_paths_dictionary.push(path.clone());
                }
            }
        }

        let info = Info::new_from_file_paths(file_paths_dictionary.clone());

        let mut this = Self {
            info,
            file_path: None,
            file_paths: Some(file_paths),
            file_paths_dictionary: Some(file_paths_dictionary),
            track_ids: None,
            track_paths: None,
            duration: 0.0,
            shuffle_schedule: Vec::new(),
            play_settings: None,
            impulse_response_spec: None,
            impulse_response_tail_db: None,
            effects: None,
        };

        this.refresh_tracks();

        this
    }

    /// Legacy constructor for backwards compatibility.
    pub fn new_from_file_paths_legacy(file_paths: &Vec<Vec<String>>) -> Self {
        let mut paths_track_list = Vec::new();
        for track in file_paths {
            paths_track_list.push(PathsTrack::new_from_file_paths(track.clone()));
        }
        Self::new_from_file_paths(paths_track_list)
    }

    // fn get_duration_from_file_path(file_path: &String) -> f64 {
    //     let file = std::fs::File::open(file_path).unwrap();
    //     let symphonia: Symphonia = Symphonia::open(file).expect("Could not open file");
    // }

    /// Rebuild the active track list (e.g., after shuffle).
    pub fn refresh_tracks(&mut self) {
        self.track_ids = None;
        self.track_paths = None;
        self.shuffle_schedule.clear();
        self.duration = 0.0;

        if let Some(file_paths) = &self.file_paths {
            let (schedule, longest_duration) = build_paths_shuffle_schedule(
                file_paths,
                &self.info,
                self.file_paths_dictionary.as_deref().unwrap_or(&[]),
            );
            self.shuffle_schedule = schedule;
            self.duration = longest_duration;

            if let Some(entry) = self.shuffle_schedule.first() {
                self.track_paths = Some(sources_to_track_paths(&entry.sources));
            }

            return;
        }

        if self.file_path.is_none() {
            return;
        }

        match self.play_settings.as_ref() {
            Some(play_settings) => match play_settings {
                PlaySettingsFile::Legacy(file) => {
                    let mut longest_duration = 0.0;
                    let mut track_index_array: Vec<u32> = Vec::new();
                    collect_legacy_tracks(
                        file.settings.inner(),
                        &mut track_index_array,
                        &mut longest_duration,
                        &self.info,
                        &mut self.duration,
                    );
                    self.track_ids = Some(track_index_array.clone());
                    self.shuffle_schedule = vec![ShuffleScheduleEntry {
                        at_ms: 0,
                        sources: track_index_array
                            .into_iter()
                            .map(ShuffleSource::TrackId)
                            .collect(),
                    }];
                }
                PlaySettingsFile::V1(file) => {
                    let (schedule, longest_duration) =
                        build_id_shuffle_schedule(&file.settings.inner().tracks, &self.info);
                    self.shuffle_schedule = schedule;
                    self.duration = longest_duration;
                }
                PlaySettingsFile::V2(file) => {
                    let (schedule, longest_duration) =
                        build_id_shuffle_schedule(&file.settings.inner().tracks, &self.info);
                    self.shuffle_schedule = schedule;
                    self.duration = longest_duration;
                }
                PlaySettingsFile::V3(file) => {
                    let (schedule, longest_duration) =
                        build_id_shuffle_schedule(&file.settings.inner().tracks, &self.info);
                    self.shuffle_schedule = schedule;
                    self.duration = longest_duration;
                }
                PlaySettingsFile::Unknown { .. } => {
                    error!("Unknown file format");
                }
            },
            None => {
                warn!("No play_settings.json found; no tracks resolved.");
            }
        }

        if let Some(entry) = self.shuffle_schedule.first() {
            self.track_ids = Some(sources_to_track_ids(&entry.sources));
        }
    }

    /// Return effects parsed from play_settings, if any.
    pub fn get_effects(&self) -> Option<Vec<AudioEffect>> {
        self.effects.clone()
    }

    fn load_play_settings(&mut self) {
        println!("Loading play settings...");
        let Some(file_path) = self.file_path.as_ref() else {
            return;
        };

        let file = std::fs::File::open(file_path).unwrap();
        let mka: Matroska = Matroska::open(file).expect("Could not open file");

        let mut parsed = None;

        for attachment in &mka.attachments {
            if attachment.name == "play_settings.json" {
                match serde_json::from_slice::<PlaySettingsFile>(&attachment.data) {
                    Ok(play_settings) => {
                        parsed = Some(play_settings);
                        break;
                    }
                    Err(err) => {
                        error!("Failed to parse play_settings.json: {}", err);
                    }
                }
            }
        }

        let Some(play_settings) = parsed else {
            return;
        };

        info!("Parsed play_settings.json");

        self.impulse_response_spec = parse_impulse_response_spec(&play_settings);
        self.impulse_response_tail_db = parse_impulse_response_tail_db(&play_settings);

        match &play_settings {
            PlaySettingsFile::V1(file) => {
                self.effects = Some(file.settings.inner().effects.clone());
            }
            PlaySettingsFile::V2(file) => {
                self.effects = Some(file.settings.inner().effects.clone());
            }
            PlaySettingsFile::V3(file) => {
                self.effects = Some(file.settings.inner().effects.clone());
            }
            _ => {}
        }

        if let Some(effects) = self.effects.as_ref() {
            info!(
                "Loaded play_settings effects ({}): {:?}",
                effects.len(),
                effects
            );
        }

        self.play_settings = Some(play_settings);
    }

    /// Get the convolution impulse response spec, if configured.
    pub fn get_impulse_response_spec(&self) -> Option<ImpulseResponseSpec> {
        self.impulse_response_spec.clone()
    }

    /// Get the configured impulse response tail trim in dB, if any.
    pub fn get_impulse_response_tail_db(&self) -> Option<f32> {
        self.impulse_response_tail_db
    }

    /// Return the container path if this is a `.prot`/`.mka` file.
    pub fn get_container_path(&self) -> Option<String> {
        self.file_path.clone()
    }

    /// Override the impulse response spec at runtime.
    pub fn set_impulse_response_spec(&mut self, spec: ImpulseResponseSpec) {
        self.impulse_response_spec = Some(spec);
    }

    /// Override the impulse response tail trim at runtime.
    pub fn set_impulse_response_tail_db(&mut self, tail_db: f32) {
        self.impulse_response_tail_db = Some(tail_db);
    }

    /// Return per-track keys for UI selection.
    pub fn get_keys(&self) -> Vec<u32> {
        // This should just be a range from 0 to the length of the track_paths or track_ids array
        if let Some(track_paths) = &self.track_paths {
            return (0..track_paths.len() as u32).collect();
        }

        if let Some(track_ids) = &self.track_ids {
            return (0..track_ids.len() as u32).collect();
        }

        Vec::new()
    }

    /// Return per-track identifiers or file paths for display.
    pub fn get_ids(&self) -> Vec<String> {
        if let Some(track_paths) = &self.track_paths {
            return track_paths.clone();
        }

        if let Some(track_ids) = &self.track_ids {
            return track_ids.iter().map(|id| format!("{}", id)).collect();
        }

        Vec::new()
    }

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

    /// Return a list of `(key, path, optional track_id)` for buffering.
    pub fn enumerated_list(&self) -> Vec<(u16, String, Option<u32>)> {
        let mut list: Vec<(u16, String, Option<u32>)> = Vec::new();
        if let Some(track_paths) = &self.track_paths {
            for (index, file_path) in track_paths.iter().enumerate() {
                list.push((index as u16, String::from(file_path), None));
            }

            return list;
        }

        if let Some(track_ids) = &self.track_ids {
            for (index, track_id) in track_ids.iter().enumerate() {
                list.push((
                    index as u16,
                    String::from(self.file_path.as_ref().unwrap()),
                    Some(*track_id),
                ));
            }

            return list;
        }

        list
    }

    /// Return container track entries for shared container streaming.
    pub fn container_track_entries(&self) -> Option<(String, Vec<(u16, u32)>)> {
        let file_path = self.file_path.as_ref()?;
        let track_ids = self.track_ids.as_ref()?;
        let mut entries = Vec::new();
        for (index, track_id) in track_ids.iter().enumerate() {
            entries.push((index as u16, *track_id));
        }
        Some((file_path.clone(), entries))
    }

    /// Get the longest selected duration (seconds).
    pub fn get_duration(&self) -> &f64 {
        &self.duration
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

        if let Some(file_paths) = self.file_paths.as_ref() {
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

        let tracks = match self.play_settings.as_ref() {
            Some(PlaySettingsFile::V1(file)) => Some(&file.settings.inner().tracks),
            Some(PlaySettingsFile::V2(file)) => Some(&file.settings.inner().tracks),
            Some(PlaySettingsFile::V3(file)) => Some(&file.settings.inner().tracks),
            _ => None,
        };

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

        if let Some(file_paths) = self.file_paths.as_mut() {
            if let Some(track) = get_paths_track_for_slot_mut(file_paths, slot_index) {
                track.level = level;
                track.pan = pan;
                return true;
            }
            return false;
        }

        match self.play_settings.as_mut() {
            Some(PlaySettingsFile::V1(file)) => update_settings_track_slot(
                file.settings.inner_mut().tracks.as_mut_slice(),
                slot_index,
                level,
                pan,
            ),
            Some(PlaySettingsFile::V2(file)) => update_settings_track_slot(
                file.settings.inner_mut().tracks.as_mut_slice(),
                slot_index,
                level,
                pan,
            ),
            Some(PlaySettingsFile::V3(file)) => update_settings_track_slot(
                file.settings.inner_mut().tracks.as_mut_slice(),
                slot_index,
                level,
                pan,
            ),
            _ => false,
        }
    }

    /// Return all slot indices that share the same track settings as `slot_index`.
    pub fn linked_slot_indices(&self, slot_index: usize) -> Option<Vec<usize>> {
        if let Some(file_paths) = self.file_paths.as_ref() {
            return linked_paths_slots(file_paths, slot_index);
        }

        let tracks = match self.play_settings.as_ref() {
            Some(PlaySettingsFile::V1(file)) => Some(&file.settings.inner().tracks),
            Some(PlaySettingsFile::V2(file)) => Some(&file.settings.inner().tracks),
            Some(PlaySettingsFile::V3(file)) => Some(&file.settings.inner().tracks),
            _ => None,
        }?;

        linked_settings_slots(tracks, slot_index)
    }

    /// Return the number of selected tracks.
    pub fn get_length(&self) -> usize {
        if let Some(track_paths) = &self.track_paths {
            return track_paths.len();
        }

        if let Some(file_paths) = &self.file_paths {
            return file_paths.len();
        }

        if let Some(track_ids) = &self.track_ids {
            return track_ids.len();
        }

        0
    }

    /// Return the number of possible unique selections based on track settings.
    pub fn count_possible_combinations(&self) -> Option<u128> {
        if let Some(file_paths) = &self.file_paths {
            return count_paths_track_combinations(file_paths);
        }

        let play_settings = self.play_settings.as_ref()?;
        match play_settings {
            PlaySettingsFile::Legacy(file) => {
                count_legacy_track_combinations(file.settings.inner())
            }
            PlaySettingsFile::V1(file) => {
                count_settings_track_combinations(&file.settings.inner().tracks)
            }
            PlaySettingsFile::V2(file) => {
                count_settings_track_combinations(&file.settings.inner().tracks)
            }
            PlaySettingsFile::V3(file) => {
                count_settings_track_combinations(&file.settings.inner().tracks)
            }
            PlaySettingsFile::Unknown { .. } => None,
        }
    }

    /// Return the unique file paths used for a multi-file container.
    pub fn get_file_paths_dictionary(&self) -> Vec<String> {
        match &self.file_paths_dictionary {
            Some(dictionary) => dictionary.to_vec(),
            None => Vec::new(),
        }
    }

    fn logical_track_slot_spans(&self) -> Vec<usize> {
        if let Some(file_paths) = self.file_paths.as_ref() {
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
            Some(PlaySettingsFile::V1(file)) => file
                .settings
                .inner()
                .tracks
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
                .collect(),
            Some(PlaySettingsFile::V2(file)) => file
                .settings
                .inner()
                .tracks
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
                .collect(),
            Some(PlaySettingsFile::V3(file)) => file
                .settings
                .inner()
                .tracks
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
                .collect(),
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

/// Standalone file-path track configuration.
#[derive(Debug, Clone)]
pub struct PathsTrack {
    /// Candidate file paths for this track.
    pub file_paths: Vec<String>,
    /// Track gain scalar.
    pub level: f32,
    /// Track pan position.
    pub pan: f32,
    /// Number of selections to pick per refresh.
    pub selections_count: u32,
    /// Timestamps where this track is reshuffled.
    pub shuffle_points: Vec<String>,
}

fn count_settings_track_combinations(tracks: &[SettingsTrack]) -> Option<u128> {
    let mut total: u128 = 1;
    for track in tracks {
        let choices = track.ids.len() as u128;
        let reshuffle_events = parse_shuffle_points(&track.shuffle_points).len() as u32;
        let total_draws = track
            .selections_count
            .checked_mul(reshuffle_events.checked_add(1)?)?;
        let count = checked_pow(choices, total_draws)?;
        total = total.checked_mul(count)?;
    }
    Some(total)
}

fn count_paths_track_combinations(tracks: &[PathsTrack]) -> Option<u128> {
    let mut total: u128 = 1;
    for track in tracks {
        let choices = track.file_paths.len() as u128;
        let reshuffle_events = parse_shuffle_points(&track.shuffle_points).len() as u32;
        let total_draws = track
            .selections_count
            .checked_mul(reshuffle_events.checked_add(1)?)?;
        let count = checked_pow(choices, total_draws)?;
        total = total.checked_mul(count)?;
    }
    Some(total)
}

fn count_legacy_track_combinations(settings: &PlaySettingsLegacy) -> Option<u128> {
    let mut total: u128 = 1;
    for track in &settings.tracks {
        let choices = track.length.unwrap_or(0) as u128;
        let count = checked_pow(choices, 1)?;
        total = total.checked_mul(count)?;
    }
    Some(total)
}

fn checked_pow(base: u128, exp: u32) -> Option<u128> {
    if exp == 0 {
        return Some(1);
    }
    if base == 0 {
        return Some(1);
    }
    let mut result: u128 = 1;
    for _ in 0..exp {
        result = result.checked_mul(base)?;
    }
    Some(result)
}

impl PathsTrack {
    /// Create a new PathsTrack from a vector of file paths.
    pub fn new_from_file_paths(file_paths: Vec<String>) -> Self {
        PathsTrack {
            file_paths,
            level: 1.0,
            pan: 0.0,
            selections_count: 1,
            shuffle_points: Vec::new(),
        }
    }
}

fn build_id_shuffle_schedule(
    tracks: &[SettingsTrack],
    info: &Info,
) -> (Vec<ShuffleScheduleEntry>, f64) {
    let mut shuffle_timestamps = BTreeSet::new();
    let mut slot_candidates: Vec<Vec<u32>> = Vec::new();
    let mut slot_points: Vec<HashSet<u64>> = Vec::new();
    let mut current_ids: Vec<u32> = Vec::new();
    let mut longest_duration = 0.0_f64;
    shuffle_timestamps.insert(0);

    for track in tracks {
        if track.ids.is_empty() {
            continue;
        }
        let selections = track.selections_count as usize;
        if selections == 0 {
            continue;
        }
        let points = parse_shuffle_points(&track.shuffle_points);
        for point in &points {
            shuffle_timestamps.insert(*point);
        }
        let point_set: HashSet<u64> = points.into_iter().collect();
        for _ in 0..selections {
            slot_candidates.push(track.ids.clone());
            slot_points.push(point_set.clone());
            let choice = random_id(&track.ids);
            if let Some(duration) = info.get_duration(choice) {
                longest_duration = longest_duration.max(duration);
            }
            current_ids.push(choice);
        }
    }

    let mut schedule = Vec::new();
    if current_ids.is_empty() {
        return (schedule, longest_duration);
    }

    schedule.push(ShuffleScheduleEntry {
        at_ms: 0,
        sources: current_ids
            .iter()
            .copied()
            .map(ShuffleSource::TrackId)
            .collect(),
    });

    for timestamp in shuffle_timestamps.into_iter().filter(|point| *point > 0) {
        for slot_index in 0..current_ids.len() {
            if slot_points[slot_index].contains(&timestamp) {
                current_ids[slot_index] = random_id(&slot_candidates[slot_index]);
                if let Some(duration) = info.get_duration(current_ids[slot_index]) {
                    longest_duration = longest_duration.max(duration);
                }
            }
        }
        schedule.push(ShuffleScheduleEntry {
            at_ms: timestamp,
            sources: current_ids
                .iter()
                .copied()
                .map(ShuffleSource::TrackId)
                .collect(),
        });
    }

    (schedule, longest_duration)
}

fn build_paths_shuffle_schedule(
    tracks: &[PathsTrack],
    info: &Info,
    dictionary: &[String],
) -> (Vec<ShuffleScheduleEntry>, f64) {
    let mut shuffle_timestamps = BTreeSet::new();
    let mut slot_candidates: Vec<Vec<String>> = Vec::new();
    let mut slot_points: Vec<HashSet<u64>> = Vec::new();
    let mut current_paths: Vec<String> = Vec::new();
    let mut longest_duration = 0.0_f64;
    let dictionary_lookup: HashMap<&str, u32> = dictionary
        .iter()
        .enumerate()
        .map(|(index, path)| (path.as_str(), index as u32))
        .collect();
    shuffle_timestamps.insert(0);

    for track in tracks {
        if track.file_paths.is_empty() {
            continue;
        }
        let selections = track.selections_count as usize;
        if selections == 0 {
            continue;
        }
        let points = parse_shuffle_points(&track.shuffle_points);
        for point in &points {
            shuffle_timestamps.insert(*point);
        }
        let point_set: HashSet<u64> = points.into_iter().collect();
        for _ in 0..selections {
            slot_candidates.push(track.file_paths.clone());
            slot_points.push(point_set.clone());
            let choice = random_path(&track.file_paths);
            if let Some(index) = dictionary_lookup.get(choice.as_str()).copied() {
                if let Some(duration) = info.get_duration(index) {
                    longest_duration = longest_duration.max(duration);
                }
            }
            current_paths.push(choice);
        }
    }

    let mut schedule = Vec::new();
    if current_paths.is_empty() {
        return (schedule, longest_duration);
    }

    schedule.push(ShuffleScheduleEntry {
        at_ms: 0,
        sources: current_paths
            .iter()
            .cloned()
            .map(ShuffleSource::FilePath)
            .collect(),
    });

    for timestamp in shuffle_timestamps.into_iter().filter(|point| *point > 0) {
        for slot_index in 0..current_paths.len() {
            if slot_points[slot_index].contains(&timestamp) {
                current_paths[slot_index] = random_path(&slot_candidates[slot_index]);
                if let Some(index) = dictionary_lookup
                    .get(current_paths[slot_index].as_str())
                    .copied()
                {
                    if let Some(duration) = info.get_duration(index) {
                        longest_duration = longest_duration.max(duration);
                    }
                }
            }
        }
        schedule.push(ShuffleScheduleEntry {
            at_ms: timestamp,
            sources: current_paths
                .iter()
                .cloned()
                .map(ShuffleSource::FilePath)
                .collect(),
        });
    }

    (schedule, longest_duration)
}

fn parse_shuffle_points(points: &[String]) -> Vec<u64> {
    let mut parsed = Vec::new();
    for point in points {
        match parse_timestamp_ms(point) {
            Some(value) => parsed.push(value),
            None => warn!("Invalid shuffle point timestamp: {}", point),
        }
    }
    parsed.sort_unstable();
    parsed.dedup();
    parsed
}

fn parse_timestamp_ms(value: &str) -> Option<u64> {
    let parts: Vec<&str> = value.trim().split(':').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }

    let seconds_component = parts.last()?.parse::<f64>().ok()?;
    if seconds_component.is_sign_negative() {
        return None;
    }

    let minutes = if parts.len() >= 2 {
        parts[parts.len() - 2].parse::<u64>().ok()?
    } else {
        0
    };
    let hours = if parts.len() == 3 {
        parts[0].parse::<u64>().ok()?
    } else {
        0
    };

    let total_seconds = (hours as f64 * 3600.0) + (minutes as f64 * 60.0) + seconds_component;
    if total_seconds.is_sign_negative() || !total_seconds.is_finite() {
        return None;
    }
    Some((total_seconds * 1000.0).round() as u64)
}

fn seconds_to_ms(seconds: f64) -> u64 {
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    (seconds * 1000.0).round() as u64
}

fn random_id(ids: &[u32]) -> u32 {
    let random_index = rand::thread_rng().gen_range(0..ids.len());
    ids[random_index]
}

fn random_path(paths: &[String]) -> String {
    let random_index = rand::thread_rng().gen_range(0..paths.len());
    paths[random_index].clone()
}

fn sanitize_level(level: f32) -> f32 {
    if level.is_finite() {
        level.max(0.0)
    } else {
        1.0
    }
}

fn sanitize_pan(pan: f32) -> f32 {
    if pan.is_finite() {
        pan.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

fn group_ids_by_slot_spans(ids: &[String], slot_spans: &[usize]) -> Vec<Vec<String>> {
    if ids.is_empty() {
        return Vec::new();
    }

    // If spans are unavailable, preserve data by treating each slot as its own track.
    if slot_spans.is_empty() {
        return ids.iter().map(|id| vec![id.clone()]).collect();
    }

    let mut grouped = Vec::new();
    let mut cursor = 0usize;
    for span in slot_spans {
        if *span == 0 || cursor >= ids.len() {
            continue;
        }
        let end = (cursor + *span).min(ids.len());
        grouped.push(ids[cursor..end].to_vec());
        cursor = end;
    }

    // Keep any extra slots visible even if schedule/config spans diverge.
    while cursor < ids.len() {
        grouped.push(vec![ids[cursor].clone()]);
        cursor += 1;
    }

    grouped
}

fn build_slot_layout(slot_count: usize, slot_spans: &[usize]) -> (Vec<(usize, usize)>, usize) {
    if slot_count == 0 {
        return (Vec::new(), 0);
    }

    let mut layout = Vec::with_capacity(slot_count);
    let mut cursor = 0usize;

    if slot_spans.is_empty() {
        for slot in 0..slot_count {
            layout.push((slot, 0));
        }
        return (layout, slot_count);
    }

    for (logical_track_index, span) in slot_spans.iter().copied().enumerate() {
        let span = span.max(1);
        for selection_index in 0..span {
            if cursor >= slot_count {
                break;
            }
            layout.push((logical_track_index, selection_index));
            cursor += 1;
        }
        if cursor >= slot_count {
            break;
        }
    }

    let mut logical_track_count = slot_spans.len();
    while cursor < slot_count {
        layout.push((logical_track_count, 0));
        logical_track_count += 1;
        cursor += 1;
    }

    (layout, logical_track_count.max(slot_spans.len()))
}

#[allow(clippy::too_many_arguments)]
fn build_segment_instance(
    instance_id: usize,
    logical_track_index: usize,
    slot_index: usize,
    selection_index: usize,
    source: &ShuffleSource,
    segment_start_ms: u64,
    segment_end_ms: Option<u64>,
    start_ms: u64,
    occurrence_counters: &mut HashMap<(usize, usize), usize>,
) -> Option<RuntimeInstanceMeta> {
    let clipped_start = segment_start_ms.max(start_ms);
    let clipped_end = segment_end_ms.map(|end| end.max(start_ms));
    if let Some(end) = clipped_end {
        if end <= clipped_start {
            return None;
        }
    }

    let relative_start = clipped_start.saturating_sub(start_ms);
    let relative_end = clipped_end.map(|end| end.saturating_sub(start_ms));
    let key = (logical_track_index, selection_index);
    let occurrence_index = occurrence_counters.get(&key).copied().unwrap_or(0);
    occurrence_counters.insert(key, occurrence_index + 1);

    Some(RuntimeInstanceMeta {
        instance_id,
        logical_track_index,
        slot_index,
        source_key: source.clone(),
        active_windows: vec![ActiveWindow {
            start_ms: relative_start,
            end_ms: relative_end,
        }],
        selection_index,
        occurrence_index,
    })
}

fn get_paths_track_for_slot_mut(
    tracks: &mut [PathsTrack],
    slot_index: usize,
) -> Option<&mut PathsTrack> {
    let mut slot_cursor = 0usize;
    for track in tracks.iter_mut() {
        let span = track.selections_count.max(1) as usize;
        if slot_index < slot_cursor + span {
            return Some(track);
        }
        slot_cursor += span;
    }
    None
}

fn update_settings_track_slot(
    tracks: &mut [SettingsTrack],
    slot_index: usize,
    level: f32,
    pan: f32,
) -> bool {
    let mut slot_cursor = 0usize;
    for track in tracks.iter_mut() {
        let span = track.selections_count.max(1) as usize;
        if slot_index < slot_cursor + span {
            track.level = level;
            track.pan = pan;
            return true;
        }
        slot_cursor += span;
    }
    false
}

fn linked_paths_slots(tracks: &[PathsTrack], slot_index: usize) -> Option<Vec<usize>> {
    let mut slot_cursor = 0usize;
    for track in tracks {
        let span = track.selections_count.max(1) as usize;
        if slot_index < slot_cursor + span {
            return Some((slot_cursor..(slot_cursor + span)).collect());
        }
        slot_cursor += span;
    }
    None
}

fn linked_settings_slots(tracks: &[SettingsTrack], slot_index: usize) -> Option<Vec<usize>> {
    let mut slot_cursor = 0usize;
    for track in tracks {
        let span = track.selections_count.max(1) as usize;
        if slot_index < slot_cursor + span {
            return Some((slot_cursor..(slot_cursor + span)).collect());
        }
        slot_cursor += span;
    }
    None
}

fn sources_to_track_ids(sources: &[ShuffleSource]) -> Vec<u32> {
    sources
        .iter()
        .filter_map(|source| match source {
            ShuffleSource::TrackId(track_id) => Some(*track_id),
            ShuffleSource::FilePath(_) => None,
        })
        .collect()
}

fn sources_to_track_paths(sources: &[ShuffleSource]) -> Vec<String> {
    sources
        .iter()
        .filter_map(|source| match source {
            ShuffleSource::TrackId(_) => None,
            ShuffleSource::FilePath(path) => Some(path.clone()),
        })
        .collect()
}

fn collect_legacy_tracks(
    settings: &PlaySettingsLegacy,
    track_index_array: &mut Vec<u32>,
    longest_duration: &mut f64,
    info: &Info,
    total_duration: &mut f64,
) {
    for track in &settings.tracks {
        let (Some(starting_index), Some(length)) = (track.starting_index, track.length) else {
            continue;
        };
        let starting_index = starting_index + 1;
        let index = rand::thread_rng().gen_range(starting_index..(starting_index + length));
        if let Some(track_duration) = info.get_duration(index) {
            if track_duration > *longest_duration {
                *longest_duration = track_duration;
                *total_duration = *longest_duration;
            }
        }
        track_index_array.push(index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_info() -> Info {
        Info {
            file_paths: Vec::new(),
            duration_map: HashMap::new(),
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
        }
    }

    fn settings_track(
        ids: Vec<u32>,
        selections_count: u32,
        shuffle_points: Vec<&str>,
    ) -> SettingsTrack {
        SettingsTrack {
            level: 1.0,
            pan: 0.0,
            ids,
            name: "Track".to_string(),
            safe_name: "Track".to_string(),
            selections_count,
            shuffle_points: shuffle_points.into_iter().map(|v| v.to_string()).collect(),
        }
    }

    #[test]
    fn count_settings_combinations_without_shuffle_points() {
        let tracks = vec![settings_track(vec![1, 2, 3], 2, vec![])];
        assert_eq!(count_settings_track_combinations(&tracks), Some(9));
    }

    #[test]
    fn count_settings_combinations_with_shuffle_points() {
        let tracks = vec![settings_track(vec![1, 2, 3], 2, vec!["0:30", "1:00"])];
        assert_eq!(count_settings_track_combinations(&tracks), Some(729));
    }

    #[test]
    fn count_settings_combinations_uses_unique_valid_shuffle_points() {
        let tracks = vec![settings_track(
            vec![1, 2, 3, 4],
            1,
            vec!["1:00", "bad", "1:00"],
        )];
        assert_eq!(count_settings_track_combinations(&tracks), Some(16));
    }

    #[test]
    fn count_paths_combinations_with_shuffle_points() {
        let tracks = vec![PathsTrack {
            file_paths: vec!["a.wav".to_string(), "b.wav".to_string()],
            level: 1.0,
            pan: 0.0,
            selections_count: 1,
            shuffle_points: vec!["0:15".to_string(), "0:45".to_string()],
        }];
        assert_eq!(count_paths_track_combinations(&tracks), Some(8));
    }

    #[test]
    fn get_track_mix_settings_repeats_by_selections_count_for_paths_tracks() {
        let prot = Prot {
            info: test_info(),
            file_path: None,
            file_paths: Some(vec![PathsTrack {
                file_paths: vec!["a.wav".to_string()],
                level: 0.7,
                pan: -0.3,
                selections_count: 2,
                shuffle_points: vec![],
            }]),
            file_paths_dictionary: Some(vec!["a.wav".to_string()]),
            track_ids: None,
            track_paths: None,
            duration: 0.0,
            shuffle_schedule: Vec::new(),
            play_settings: None,
            impulse_response_spec: None,
            impulse_response_tail_db: None,
            effects: None,
        };

        let settings = prot.get_track_mix_settings();
        assert_eq!(settings.get(&0), Some(&(0.7, -0.3)));
        assert_eq!(settings.get(&1), Some(&(0.7, -0.3)));
    }

    #[test]
    fn set_slot_mix_settings_updates_paths_track() {
        let mut prot = Prot {
            info: test_info(),
            file_path: None,
            file_paths: Some(vec![PathsTrack {
                file_paths: vec!["a.wav".to_string()],
                level: 1.0,
                pan: 0.0,
                selections_count: 2,
                shuffle_points: vec![],
            }]),
            file_paths_dictionary: Some(vec!["a.wav".to_string()]),
            track_ids: None,
            track_paths: None,
            duration: 0.0,
            shuffle_schedule: Vec::new(),
            play_settings: None,
            impulse_response_spec: None,
            impulse_response_tail_db: None,
            effects: None,
        };

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
            file_path: Some("dummy.prot".to_string()),
            file_paths: None,
            file_paths_dictionary: None,
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
        let prot = Prot {
            info: test_info(),
            file_path: None,
            file_paths: Some(vec![
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
            ]),
            file_paths_dictionary: Some(vec!["a.wav".to_string(), "b.wav".to_string()]),
            track_ids: None,
            track_paths: None,
            duration: 0.0,
            shuffle_schedule: Vec::new(),
            play_settings: None,
            impulse_response_spec: None,
            impulse_response_tail_db: None,
            effects: None,
        };

        assert_eq!(prot.linked_slot_indices(0), Some(vec![0, 1]));
        assert_eq!(prot.linked_slot_indices(1), Some(vec![0, 1]));
        assert_eq!(prot.linked_slot_indices(2), Some(vec![2]));
        assert_eq!(prot.linked_slot_indices(3), None);
    }

    #[test]
    fn group_ids_by_slot_spans_groups_multiple_selections_per_logical_track() {
        let ids = vec![
            "a1".to_string(),
            "a2".to_string(),
            "b1".to_string(),
            "c1".to_string(),
            "c2".to_string(),
            "c3".to_string(),
        ];
        let grouped = group_ids_by_slot_spans(&ids, &[2, 1, 3]);
        assert_eq!(
            grouped,
            vec![
                vec!["a1".to_string(), "a2".to_string()],
                vec!["b1".to_string()],
                vec!["c1".to_string(), "c2".to_string(), "c3".to_string()],
            ]
        );
    }

    #[test]
    fn get_shuffle_schedule_groups_by_paths_track_selections_count() {
        let prot = Prot {
            info: test_info(),
            file_path: None,
            file_paths: Some(vec![
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
            ]),
            file_paths_dictionary: Some(vec![
                "a.wav".to_string(),
                "b.wav".to_string(),
                "c.wav".to_string(),
            ]),
            track_ids: None,
            track_paths: None,
            duration: 0.0,
            shuffle_schedule: vec![ShuffleScheduleEntry {
                at_ms: 0,
                sources: vec![
                    ShuffleSource::FilePath("a.wav".to_string()),
                    ShuffleSource::FilePath("b.wav".to_string()),
                    ShuffleSource::FilePath("c.wav".to_string()),
                ],
            }],
            play_settings: None,
            impulse_response_spec: None,
            impulse_response_tail_db: None,
            effects: None,
        };

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

    #[test]
    fn build_runtime_instance_plan_keeps_duplicate_instances() {
        let prot = Prot {
            info: test_info(),
            file_path: Some("demo.prot".to_string()),
            file_paths: None,
            file_paths_dictionary: None,
            track_ids: None,
            track_paths: None,
            duration: 0.0,
            shuffle_schedule: vec![
                ShuffleScheduleEntry {
                    at_ms: 0,
                    sources: vec![
                        ShuffleSource::TrackId(1),
                        ShuffleSource::TrackId(2),
                        ShuffleSource::TrackId(3),
                    ],
                },
                ShuffleScheduleEntry {
                    at_ms: 14_604,
                    sources: vec![
                        ShuffleSource::TrackId(2),
                        ShuffleSource::TrackId(2),
                        ShuffleSource::TrackId(2),
                    ],
                },
            ],
            play_settings: Some(PlaySettingsFile::V1(
                crate::container::play_settings::PlaySettingsV1File {
                    settings: crate::container::play_settings::PlaySettingsContainer::Flat(
                        crate::container::play_settings::PlaySettingsV1 {
                            effects: Vec::new(),
                            tracks: vec![
                                SettingsTrack {
                                    level: 1.0,
                                    pan: 0.0,
                                    ids: vec![1, 2],
                                    name: "A".to_string(),
                                    safe_name: "a".to_string(),
                                    selections_count: 2,
                                    shuffle_points: vec!["0:14.604".to_string()],
                                },
                                SettingsTrack {
                                    level: 1.0,
                                    pan: 0.0,
                                    ids: vec![2, 3],
                                    name: "B".to_string(),
                                    safe_name: "b".to_string(),
                                    selections_count: 1,
                                    shuffle_points: vec!["0:14.604".to_string()],
                                },
                            ],
                        },
                    ),
                },
            )),
            impulse_response_spec: None,
            impulse_response_tail_db: None,
            effects: None,
        };

        let plan = prot.build_runtime_instance_plan(0.0);
        assert_eq!(plan.logical_track_count, 2);
        assert_eq!(plan.instances.len(), 5);

        let logical0 = plan
            .instances
            .iter()
            .filter(|instance| instance.logical_track_index == 0)
            .count();
        let logical1 = plan
            .instances
            .iter()
            .filter(|instance| instance.logical_track_index == 1)
            .count();
        assert_eq!(logical0, 3);
        assert_eq!(logical1, 2);
    }

    #[test]
    fn build_runtime_instance_plan_clips_windows_to_start_time() {
        let prot = Prot {
            info: test_info(),
            file_path: Some("demo.prot".to_string()),
            file_paths: None,
            file_paths_dictionary: None,
            track_ids: Some(vec![1]),
            track_paths: None,
            duration: 0.0,
            shuffle_schedule: vec![
                ShuffleScheduleEntry {
                    at_ms: 0,
                    sources: vec![ShuffleSource::TrackId(1)],
                },
                ShuffleScheduleEntry {
                    at_ms: 10_000,
                    sources: vec![ShuffleSource::TrackId(2)],
                },
            ],
            play_settings: None,
            impulse_response_spec: None,
            impulse_response_tail_db: None,
            effects: None,
        };

        let plan = prot.build_runtime_instance_plan(5.0);
        assert_eq!(plan.event_boundaries_ms, vec![5_000]);
        assert_eq!(plan.instances.len(), 2);
        assert_eq!(plan.instances[0].active_windows[0].start_ms, 0);
        assert_eq!(plan.instances[0].active_windows[0].end_ms, Some(5_000));
        assert_eq!(plan.instances[1].active_windows[0].start_ms, 5_000);
    }

    #[test]
    fn parse_timestamp_ms_parses_common_formats() {
        assert_eq!(parse_timestamp_ms("1:02.500"), Some(62_500));
        assert_eq!(parse_timestamp_ms("90"), Some(90_000));
        assert_eq!(parse_timestamp_ms("bad"), None);
    }

    #[test]
    fn sanitize_helpers_clamp_ranges() {
        assert_eq!(sanitize_level(-1.0), 0.0);
        assert_eq!(sanitize_level(3.0), 2.0);
        assert_eq!(sanitize_pan(-2.0), -1.0);
        assert_eq!(sanitize_pan(2.0), 1.0);
    }
}
