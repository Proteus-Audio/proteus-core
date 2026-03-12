use super::super::{GetPeaksOptions, PeakWindow, PeaksData};
use super::io::write_peaks_file;
use super::read_peaks_with_options;

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn test_file_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    std::env::temp_dir().join(format!("proteus-peaks-{}.bin", nanos))
}

#[test]
fn round_trips_full_peaks_file() {
    let path = test_file_path();
    let data = PeaksData {
        sample_rate: 48_000,
        window_size: 480,
        channels: vec![
            vec![
                PeakWindow {
                    max: 0.5,
                    min: -0.5,
                },
                PeakWindow {
                    max: 0.2,
                    min: -0.1,
                },
            ],
            vec![
                PeakWindow {
                    max: 0.4,
                    min: -0.4,
                },
                PeakWindow {
                    max: 0.1,
                    min: -0.2,
                },
            ],
        ],
    };

    write_peaks_file(path.to_str().unwrap(), &data).expect("write");
    let read_back =
        read_peaks_with_options(path.to_str().unwrap(), &GetPeaksOptions::default()).expect("read");

    assert_eq!(read_back.sample_rate, 48_000);
    assert_eq!(read_back.window_size, 480);
    assert_eq!(read_back.channels.len(), 2);
    assert_eq!(read_back.channels[0].len(), 2);
    assert_eq!(read_back.channels[0][0].max, 0.5);
    assert_eq!(read_back.channels[1][1].min, -0.2);

    let _ = std::fs::remove_file(path);
}

#[test]
fn reads_peak_range() {
    let path = test_file_path();
    let data = PeaksData {
        sample_rate: 10,
        window_size: 2,
        channels: vec![vec![
            PeakWindow {
                max: 1.0,
                min: -1.0,
            },
            PeakWindow {
                max: 2.0,
                min: -2.0,
            },
            PeakWindow {
                max: 3.0,
                min: -3.0,
            },
        ]],
    };

    write_peaks_file(path.to_str().unwrap(), &data).expect("write");
    let slice = read_peaks_with_options(
        path.to_str().unwrap(),
        &GetPeaksOptions {
            start_seconds: Some(0.2),
            end_seconds: Some(0.6),
            ..Default::default()
        },
    )
    .expect("range");

    assert_eq!(slice.channels.len(), 1);
    assert_eq!(slice.channels[0].len(), 2);
    assert_eq!(slice.channels[0][0].max, 2.0);
    assert_eq!(slice.channels[0][1].max, 3.0);

    let _ = std::fs::remove_file(path);
}

#[test]
fn reads_with_options_channel_limit_and_reduction() {
    let path = test_file_path();
    let data = PeaksData {
        sample_rate: 20,
        window_size: 1,
        channels: vec![
            vec![
                PeakWindow {
                    max: 1.0,
                    min: -1.0,
                },
                PeakWindow {
                    max: 3.0,
                    min: -3.0,
                },
                PeakWindow {
                    max: 5.0,
                    min: -5.0,
                },
                PeakWindow {
                    max: 7.0,
                    min: -7.0,
                },
            ],
            vec![
                PeakWindow {
                    max: 10.0,
                    min: -10.0,
                },
                PeakWindow {
                    max: 20.0,
                    min: -20.0,
                },
                PeakWindow {
                    max: 30.0,
                    min: -30.0,
                },
                PeakWindow {
                    max: 40.0,
                    min: -40.0,
                },
            ],
        ],
    };

    write_peaks_file(path.to_str().unwrap(), &data).expect("write");
    let slice = read_peaks_with_options(
        path.to_str().unwrap(),
        &GetPeaksOptions {
            start_seconds: Some(0.0),
            end_seconds: Some(0.2),
            target_peaks: Some(2),
            channels: Some(1),
        },
    )
    .expect("read with options");

    assert_eq!(slice.channels.len(), 1);
    assert_eq!(slice.channels[0].len(), 2);
    assert_eq!(slice.channels[0][0].max, 2.0); // average of 1.0 and 3.0
    assert_eq!(slice.channels[0][1].max, 6.0); // average of 5.0 and 7.0

    let _ = std::fs::remove_file(path);
}

#[test]
fn returns_all_when_target_larger_than_available() {
    let path = test_file_path();
    let data = PeaksData {
        sample_rate: 10,
        window_size: 2,
        channels: vec![vec![
            PeakWindow {
                max: 1.0,
                min: -1.0,
            },
            PeakWindow {
                max: 2.0,
                min: -2.0,
            },
        ]],
    };

    write_peaks_file(path.to_str().unwrap(), &data).expect("write");
    let slice = read_peaks_with_options(
        path.to_str().unwrap(),
        &GetPeaksOptions {
            start_seconds: Some(0.0),
            end_seconds: Some(1.0),
            target_peaks: Some(10),
            channels: Some(1),
        },
    )
    .expect("read with options");

    assert_eq!(slice.channels.len(), 1);
    assert_eq!(slice.channels[0].len(), 10);
    assert_eq!(slice.channels[0][0].max, 1.0);
    assert_eq!(slice.channels[0][1].max, 1.0);
    assert_eq!(slice.channels[0][2].max, 2.0);
    assert_eq!(slice.channels[0][3].max, 2.0);
    assert_eq!(slice.channels[0][4].max, 0.0);
    assert_eq!(slice.channels[0][9].max, 0.0);

    let _ = std::fs::remove_file(path);
}

#[test]
fn zero_pads_when_requested_range_is_beyond_audio() {
    let path = test_file_path();
    let data = PeaksData {
        sample_rate: 10,
        window_size: 2,
        channels: vec![vec![
            PeakWindow {
                max: 1.0,
                min: -1.0,
            },
            PeakWindow {
                max: 2.0,
                min: -2.0,
            },
        ]],
    };

    write_peaks_file(path.to_str().unwrap(), &data).expect("write");
    let slice = read_peaks_with_options(
        path.to_str().unwrap(),
        &GetPeaksOptions {
            start_seconds: Some(1.0),
            end_seconds: Some(2.0),
            target_peaks: Some(4),
            channels: Some(1),
        },
    )
    .expect("read with options");

    assert_eq!(slice.channels.len(), 1);
    assert_eq!(slice.channels[0].len(), 4);
    for peak in &slice.channels[0] {
        assert_eq!(peak.max, 0.0);
        assert_eq!(peak.min, 0.0);
    }

    let _ = std::fs::remove_file(path);
}
