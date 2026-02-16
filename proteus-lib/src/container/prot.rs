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

#[derive(Debug, Clone)]
pub(crate) struct ShuffleRuntimePlan {
    pub current_sources: Vec<ShuffleSource>,
    pub upcoming_events: Vec<ShuffleScheduleEntry>,
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
    pub fn new(file_path: &String) -> Self {
        let info = Info::new(file_path.clone());

        println!("Info: {:?}", info);

        let mut this = Self {
            info,
            file_path: Some(file_path.clone()),
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

        if !self.file_path.is_some() {
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
            return track_ids.into_iter().map(|id| format!("{}", id)).collect();
        }

        Vec::new()
    }

    /// Return the full timestamped shuffle schedule for display.
    ///
    /// Each entry is `(time_seconds, selected_ids_or_paths)`.
    pub fn get_shuffle_schedule(&self) -> Vec<(f64, Vec<String>)> {
        if self.shuffle_schedule.is_empty() {
            let current = self.get_ids();
            if current.is_empty() {
                return Vec::new();
            }
            return vec![(0.0, current)];
        }

        self.shuffle_schedule
            .iter()
            .map(|entry| {
                let ids = entry
                    .sources
                    .iter()
                    .map(|source| match source {
                        ShuffleSource::TrackId(track_id) => track_id.to_string(),
                        ShuffleSource::FilePath(path) => path.clone(),
                    })
                    .collect();
                (entry.at_ms as f64 / 1000.0, ids)
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

    pub(crate) fn build_runtime_shuffle_plan(&self, start_time: f64) -> ShuffleRuntimePlan {
        if self.shuffle_schedule.is_empty() {
            let mut sources = Vec::new();
            if let Some(track_ids) = &self.track_ids {
                sources.extend(track_ids.iter().copied().map(ShuffleSource::TrackId));
            } else if let Some(track_paths) = &self.track_paths {
                sources.extend(track_paths.iter().cloned().map(ShuffleSource::FilePath));
            }
            return ShuffleRuntimePlan {
                current_sources: sources,
                upcoming_events: Vec::new(),
            };
        }

        let start_ms = seconds_to_ms(start_time);
        let mut current_index = 0usize;
        for (index, entry) in self.shuffle_schedule.iter().enumerate() {
            if entry.at_ms <= start_ms {
                current_index = index;
            } else {
                break;
            }
        }

        ShuffleRuntimePlan {
            current_sources: self.shuffle_schedule[current_index].sources.clone(),
            upcoming_events: self.shuffle_schedule[(current_index + 1)..].to_vec(),
        }
    }

    /// Return per-track `(level, pan)` settings keyed by track key.
    pub fn get_track_mix_settings(&self) -> HashMap<u16, (f32, f32)> {
        let mut settings = HashMap::new();

        let tracks = match self.play_settings.as_ref() {
            Some(PlaySettingsFile::V1(file)) => Some(&file.settings.inner().tracks),
            Some(PlaySettingsFile::V2(file)) => Some(&file.settings.inner().tracks),
            _ => None,
        };

        if let Some(tracks) = tracks {
            for (index, track) in tracks.iter().enumerate() {
                settings.insert(index as u16, (track.level, track.pan));
            }
        }

        settings
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
}
