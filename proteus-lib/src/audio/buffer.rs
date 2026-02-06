//! Shared ring buffers used for per-track audio staging.

use dasp_ring_buffer::Bounded;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// Shared ring buffer for a single track's samples.
pub type TrackBuffer = Arc<Mutex<Bounded<Vec<f32>>>>;
/// Mapping from track key to its shared ring buffer.
pub type TrackBufferMap = Arc<Mutex<HashMap<u16, TrackBuffer>>>;

/// Initialize an empty track buffer map.
///
/// Each track receives its own bounded ring buffer at runtime.
pub fn init_buffer_map() -> TrackBufferMap {
    let track_buffers: TrackBufferMap = Arc::new(Mutex::new(HashMap::new()));
    track_buffers
}

/// Return the remaining capacity for a track buffer.
///
/// If the track buffer has not been created yet, this returns `0`.
pub fn buffer_remaining_space(
    track_buffers: &TrackBufferMap,
    track_key: u16,
) -> usize {
    let track_buffers = track_buffers.lock().unwrap();
    match track_buffers.get(&track_key) {
        Some(track_buffer) => {
            let track_buffer = track_buffer.lock().unwrap();
            track_buffer.max_len().saturating_sub(track_buffer.len())
        }
        None => 0,
    }
}
