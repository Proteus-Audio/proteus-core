use dasp_ring_buffer::Bounded;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub fn init_buffer_map() -> Arc<Mutex<HashMap<u16, Bounded<Vec<f32>>>>> {
    let track_buffers: Arc<Mutex<HashMap<u16, Bounded<Vec<f32>>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    track_buffers
}

pub fn buffer_remaining_space(
    track_buffers: &Arc<Mutex<HashMap<u16, Bounded<Vec<f32>>>>>,
    track_key: u16,
) -> usize {
    let track_buffers = track_buffers.lock().unwrap();
    let remaining_space: usize;
    match track_buffers.get(&track_key) {
        Some(track_buffer) => {
            remaining_space = track_buffer.max_len() - track_buffer.len();
        }
        None => remaining_space = 0,
    };
    drop(track_buffers);
    remaining_space
}
