//! Shared ring buffers used for per-track audio staging.

use dasp_ring_buffer::Bounded;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// Shared ring buffer for a single track's samples.
pub(crate) type TrackBuffer = Arc<Mutex<Bounded<Vec<f32>>>>;
/// Mapping from track key to its shared ring buffer.
pub(crate) type TrackBufferMap = Arc<Mutex<HashMap<u16, TrackBuffer>>>;

/// Initialize an empty track buffer map.
///
/// Each track receives its own bounded ring buffer at runtime.
pub(crate) fn init_buffer_map() -> TrackBufferMap {
    let track_buffers: TrackBufferMap = Arc::new(Mutex::new(HashMap::new()));
    track_buffers
}

#[cfg(test)]
mod tests {
    use super::init_buffer_map;

    #[test]
    fn init_buffer_map_creates_empty_map() {
        let map = init_buffer_map();
        let inner = map.lock().unwrap();
        assert!(inner.is_empty());
    }
}
