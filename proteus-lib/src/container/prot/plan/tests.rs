use std::collections::HashMap;

use crate::container::info::Info;
use crate::container::play_settings::PlaySettingsFile;
use crate::container::play_settings::SettingsTrack;
use crate::container::prot::types::PathsTrack;
use crate::container::prot::{Prot, ProtSource, ShuffleScheduleEntry, ShuffleSource};

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
