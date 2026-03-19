use std::collections::HashMap;

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
fn build_runtime_instance_plan_keeps_duplicate_instances() {
    let mut prot = prot_from_container("demo.prot");
    prot.shuffle_schedule = vec![
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
    ];
    prot.play_settings = Some(PlaySettingsFile::V1(
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
    ));

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
    let mut prot = prot_from_container("demo.prot");
    prot.track_ids = Some(vec![1]);
    prot.shuffle_schedule = vec![
        ShuffleScheduleEntry {
            at_ms: 0,
            sources: vec![ShuffleSource::TrackId(1)],
        },
        ShuffleScheduleEntry {
            at_ms: 10_000,
            sources: vec![ShuffleSource::TrackId(2)],
        },
    ];

    let plan = prot.build_runtime_instance_plan(5.0);
    assert_eq!(plan.event_boundaries_ms, vec![5_000]);
    assert_eq!(plan.instances.len(), 2);
    assert_eq!(plan.instances[0].active_windows[0].start_ms, 0);
    assert_eq!(plan.instances[0].active_windows[0].end_ms, Some(5_000));
    assert_eq!(plan.instances[1].active_windows[0].start_ms, 5_000);
}
