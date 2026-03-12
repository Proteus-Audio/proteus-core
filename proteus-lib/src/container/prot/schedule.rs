//! Shuffle schedule construction and timestamp parsing.

use std::collections::{BTreeSet, HashSet};

use log::warn;
use rand::Rng;

use crate::container::info::Info;
use crate::container::play_settings::SettingsTrack;

use super::types::{PathsTrack, ShuffleScheduleEntry, ShuffleSource};

pub(super) fn build_id_shuffle_schedule(
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

pub(super) fn build_paths_shuffle_schedule(
    tracks: &[PathsTrack],
    info: &Info,
    dictionary: &[String],
) -> (Vec<ShuffleScheduleEntry>, f64) {
    let mut shuffle_timestamps = BTreeSet::new();
    let mut slot_candidates: Vec<Vec<String>> = Vec::new();
    let mut slot_points: Vec<HashSet<u64>> = Vec::new();
    let mut current_paths: Vec<String> = Vec::new();
    let mut longest_duration = 0.0_f64;
    let dictionary_lookup: std::collections::HashMap<&str, u32> = dictionary
        .iter()
        .enumerate()
        .map(|(index, path)| (path.as_str(), index as u32))
        .collect();
    shuffle_timestamps.insert(0);

    for track in tracks {
        longest_duration = append_path_track_slots(
            track,
            info,
            &dictionary_lookup,
            &mut shuffle_timestamps,
            &mut slot_candidates,
            &mut slot_points,
            &mut current_paths,
            longest_duration,
        );
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

#[allow(clippy::too_many_arguments)]
fn append_path_track_slots(
    track: &PathsTrack,
    info: &Info,
    dictionary_lookup: &std::collections::HashMap<&str, u32>,
    shuffle_timestamps: &mut BTreeSet<u64>,
    slot_candidates: &mut Vec<Vec<String>>,
    slot_points: &mut Vec<HashSet<u64>>,
    current_paths: &mut Vec<String>,
    mut longest_duration: f64,
) -> f64 {
    if track.file_paths.is_empty() {
        return longest_duration;
    }

    let selections = track.selections_count as usize;
    if selections == 0 {
        return longest_duration;
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
        longest_duration =
            update_longest_duration_for_path(info, dictionary_lookup, &choice, longest_duration);
        current_paths.push(choice);
    }

    longest_duration
}

fn update_longest_duration_for_path(
    info: &Info,
    dictionary_lookup: &std::collections::HashMap<&str, u32>,
    path: &str,
    longest_duration: f64,
) -> f64 {
    dictionary_lookup
        .get(path)
        .copied()
        .and_then(|index| info.get_duration(index))
        .map(|duration| longest_duration.max(duration))
        .unwrap_or(longest_duration)
}

pub(super) fn parse_shuffle_points(points: &[String]) -> Vec<u64> {
    let mut parsed = Vec::new();
    for point in points {
        match parse_timestamp_ms(point) {
            Some(value) => parsed.push(value),
            None => warn!("invalid shuffle point timestamp: {}", point),
        }
    }
    parsed.sort_unstable();
    parsed.dedup();
    parsed
}

fn parse_timestamp_ms(value: &str) -> Option<u64> {
    let parts: Vec<&str> = value.trim().split(':').collect();
    if parts.len() > 3 {
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

pub(super) fn seconds_to_ms(seconds: f64) -> u64 {
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    (seconds * 1000.0).round() as u64
}

pub(super) fn random_id(ids: &[u32]) -> u32 {
    let random_index = rand::thread_rng().gen_range(0..ids.len());
    ids[random_index]
}

pub(super) fn random_path(paths: &[String]) -> String {
    let random_index = rand::thread_rng().gen_range(0..paths.len());
    paths[random_index].clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_timestamp_ms_parses_common_formats() {
        assert_eq!(parse_timestamp_ms("1:02.500"), Some(62_500));
        assert_eq!(parse_timestamp_ms("90"), Some(90_000));
        assert_eq!(parse_timestamp_ms("bad"), None);
    }
}
