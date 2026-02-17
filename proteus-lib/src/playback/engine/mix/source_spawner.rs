//! Source-track spawning helpers for the mix runtime.

use dasp_ring_buffer::Bounded;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::audio::buffer::TrackBuffer;
use crate::container::prot::ShuffleSource;
use crate::track::{buffer_track, TrackArgs};

/// Helper that encapsulates all state needed to spawn shuffle sources.
pub(super) struct SourceSpawner {
    pub(super) buffer_map: Arc<Mutex<HashMap<u16, TrackBuffer>>>,
    pub(super) buffer_notify: Arc<std::sync::Condvar>,
    pub(super) track_weights: Arc<Mutex<HashMap<u16, f32>>>,
    pub(super) track_channel_gains: Arc<Mutex<HashMap<u16, Vec<f32>>>>,
    pub(super) finished_tracks: Arc<Mutex<Vec<u16>>>,
    pub(super) abort: Arc<AtomicBool>,
    pub(super) container_path: Option<String>,
    pub(super) track_buffer_size: usize,
    pub(super) output_channels: u8,
    pub(super) fallback_channel_gains: Vec<Vec<f32>>,
}

impl SourceSpawner {
    /// Spawn one source into a specific slot/key pair.
    ///
    /// # Arguments
    ///
    /// * `slot_index` - Source slot index in the shuffle schedule.
    /// * `track_key` - Runtime key used to index ring buffers.
    /// * `source` - Source selector (`TrackId` or direct file path).
    /// * `event_seconds` - Source-relative start time in seconds.
    pub(super) fn spawn(
        &self,
        slot_index: usize,
        track_key: u16,
        source: &ShuffleSource,
        event_seconds: f64,
    ) {
        {
            let mut map = self.buffer_map.lock().unwrap();
            map.insert(
                track_key,
                Arc::new(Mutex::new(Bounded::from(vec![0.0; self.track_buffer_size]))),
            );
        }
        {
            let mut weights = self.track_weights.lock().unwrap();
            weights.insert(track_key, 1.0);
        }
        {
            let mut gains = self.track_channel_gains.lock().unwrap();
            gains.insert(
                track_key,
                self.fallback_channel_gains
                    .get(slot_index)
                    .cloned()
                    .unwrap_or_else(|| vec![1.0; self.output_channels.max(1) as usize]),
            );
        }

        let track_args = match source {
            ShuffleSource::TrackId(track_id) => {
                let Some(container_path) = self.container_path.as_ref() else {
                    return;
                };
                TrackArgs {
                    file_path: container_path.clone(),
                    track_id: Some(*track_id),
                    track_key,
                    buffer_map: self.buffer_map.clone(),
                    buffer_notify: Some(self.buffer_notify.clone()),
                    track_weights: None,
                    finished_tracks: self.finished_tracks.clone(),
                    start_time: event_seconds,
                    channels: self.output_channels,
                }
            }
            ShuffleSource::FilePath(path) => TrackArgs {
                file_path: path.clone(),
                track_id: None,
                track_key,
                buffer_map: self.buffer_map.clone(),
                buffer_notify: Some(self.buffer_notify.clone()),
                track_weights: None,
                finished_tracks: self.finished_tracks.clone(),
                start_time: event_seconds,
                channels: self.output_channels,
            },
        };

        buffer_track(track_args, self.abort.clone());
    }
}
