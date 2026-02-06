//! Ring-buffer helpers for track sample delivery.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::audio::buffer::{buffer_remaining_space, TrackBufferMap};

/// Push samples into the per-track ring buffer, blocking when full.
pub fn add_samples_to_buffer_map(
    buffer_map: &mut TrackBufferMap,
    track_key: u16,
    samples: Vec<f32>,
    notify: Option<&Arc<std::sync::Condvar>>,
) {
    let mut offset = 0usize;
    while offset < samples.len() {
        let map = buffer_map.lock().unwrap();
        let remaining = match map.get(&track_key) {
            Some(buffer) => {
                let buffer = buffer.lock().unwrap();
                buffer.max_len().saturating_sub(buffer.len())
            }
            None => 0,
        };

        if remaining == 0 {
            if let Some(notify) = notify {
                let (guard, _) = notify.wait_timeout(map, Duration::from_millis(20)).unwrap();
                drop(guard);
            } else {
                drop(map);
                thread::sleep(Duration::from_millis(100));
            }
            continue;
        }

        let take = remaining.min(samples.len() - offset);
        if let Some(buffer) = map.get(&track_key) {
            let mut buffer = buffer.lock().unwrap();
            for sample in samples[offset..offset + take].iter().copied() {
                buffer.push(sample);
            }
        }
        offset += take;
        drop(map);

        if let Some(notify) = notify {
            notify.notify_one();
        }
    }
}

/// Push samples into the per-track ring buffer without blocking.
#[allow(dead_code)]
pub fn add_samples_to_buffer_map_nonblocking(
    buffer_map: &mut TrackBufferMap,
    track_key: u16,
    samples: Vec<f32>,
    notify: Option<&Arc<std::sync::Condvar>>,
) {
    let remaining = buffer_remaining_space(buffer_map, track_key);
    if remaining == 0 {
        return;
    }

    let take = remaining.min(samples.len());
    let map = buffer_map.lock().unwrap();
    if let Some(buffer) = map.get(&track_key) {
        let mut buffer = buffer.lock().unwrap();
        for sample in samples.into_iter().take(take) {
            buffer.push(sample);
        }
    }
    if let Some(notify) = notify {
        notify.notify_one();
    }
}

/// Record a track key as finished (idempotent).
pub fn mark_track_as_finished(finished_tracks: &mut Arc<Mutex<Vec<u16>>>, track_key: u16) {
    let mut finished_tracks_copy = finished_tracks.lock().unwrap();
    if finished_tracks_copy.contains(&track_key) {
        return;
    }
    finished_tracks_copy.push(track_key);
    drop(finished_tracks_copy);
    log::info!("track finished: key={}", track_key);
}
