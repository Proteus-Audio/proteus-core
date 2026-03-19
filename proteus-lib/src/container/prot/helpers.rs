//! Helper functions for track slot management, sanitization, and counting.

use std::collections::HashMap;

use crate::container::info::Info;
use crate::container::play_settings::{PlaySettingsLegacy, SettingsTrack};
use crate::dsp::guardrails::sanitize_finite_clamped;

use super::schedule::parse_shuffle_points;
use super::types::{
    ActiveWindow, PathsTrack, RuntimeInstanceMeta, SegmentRange, ShuffleSource, SlotPlacement,
};

pub(super) fn sanitize_level(level: f32) -> f32 {
    sanitize_finite_clamped(level, 1.0, 0.0, 2.0)
}

pub(super) fn sanitize_pan(pan: f32) -> f32 {
    sanitize_finite_clamped(pan, 0.0, -1.0, 1.0)
}

pub(super) fn group_ids_by_slot_spans(ids: &[String], slot_spans: &[usize]) -> Vec<Vec<String>> {
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

pub(super) fn build_slot_layout(
    slot_count: usize,
    slot_spans: &[usize],
) -> (Vec<(usize, usize)>, usize) {
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

pub(super) fn build_segment_instance(
    instance_id: usize,
    placement: &SlotPlacement,
    source: &ShuffleSource,
    range: SegmentRange,
    start_ms: u64,
    occurrence_counters: &mut HashMap<(usize, usize), usize>,
) -> Option<RuntimeInstanceMeta> {
    let clipped_start = range.start_ms.max(start_ms);
    let clipped_end = range.end_ms.map(|end| end.max(start_ms));
    if let Some(end) = clipped_end {
        if end <= clipped_start {
            return None;
        }
    }

    let relative_start = clipped_start.saturating_sub(start_ms);
    let relative_end = clipped_end.map(|end| end.saturating_sub(start_ms));
    let key = (placement.logical_track_index, placement.selection_index);
    let occurrence_index = occurrence_counters.get(&key).copied().unwrap_or(0);
    occurrence_counters.insert(key, occurrence_index + 1);

    Some(RuntimeInstanceMeta {
        instance_id,
        logical_track_index: placement.logical_track_index,
        slot_index: placement.slot_index,
        source_key: source.clone(),
        active_windows: vec![ActiveWindow {
            start_ms: relative_start,
            end_ms: relative_end,
        }],
        selection_index: placement.selection_index,
        occurrence_index,
    })
}

pub(super) fn get_paths_track_for_slot_mut(
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

pub(super) fn update_settings_track_slot(
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

pub(super) fn linked_paths_slots(tracks: &[PathsTrack], slot_index: usize) -> Option<Vec<usize>> {
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

pub(super) fn linked_settings_slots(
    tracks: &[SettingsTrack],
    slot_index: usize,
) -> Option<Vec<usize>> {
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

pub(super) fn sources_to_track_ids(sources: &[ShuffleSource]) -> Vec<u32> {
    sources
        .iter()
        .filter_map(|source| match source {
            ShuffleSource::TrackId(track_id) => Some(*track_id),
            ShuffleSource::FilePath(_) => None,
        })
        .collect()
}

pub(super) fn sources_to_track_paths(sources: &[ShuffleSource]) -> Vec<String> {
    sources
        .iter()
        .filter_map(|source| match source {
            ShuffleSource::TrackId(_) => None,
            ShuffleSource::FilePath(path) => Some(path.clone()),
        })
        .collect()
}

pub(super) fn collect_legacy_tracks(
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

pub(super) fn count_settings_track_combinations(tracks: &[SettingsTrack]) -> Option<u128> {
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

pub(super) fn count_paths_track_combinations(tracks: &[PathsTrack]) -> Option<u128> {
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

pub(super) fn count_legacy_track_combinations(settings: &PlaySettingsLegacy) -> Option<u128> {
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

use rand::Rng;

#[cfg(test)]
mod tests {
    use super::*;

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
    fn sanitize_helpers_clamp_ranges() {
        assert_eq!(sanitize_level(-1.0), 0.0);
        assert_eq!(sanitize_level(3.0), 2.0);
        assert_eq!(sanitize_pan(-2.0), -1.0);
        assert_eq!(sanitize_pan(2.0), 1.0);
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
}
