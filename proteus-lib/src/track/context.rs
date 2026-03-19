//! Shared decode-context for legacy track buffering workers.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Condvar, Mutex};

use crate::audio::buffer::TrackBufferMap;

/// Shared I/O and synchronization state passed to decode helpers.
///
/// Bundles the buffer map, notification condvar, abort signal, and
/// finished-track bookkeeping that every decode worker needs.
pub(crate) struct DecodeContext {
    /// Shared ring-buffer map that decoded samples are pushed into.
    pub buffer_map: TrackBufferMap,
    /// Optional condvar notified when new samples are available or a track finishes.
    pub buffer_notify: Option<Arc<Condvar>>,
    /// Abort signal checked each iteration to stop early.
    pub abort: Arc<AtomicBool>,
    /// Shared finished-track list for downstream bookkeeping.
    pub finished_tracks: Arc<Mutex<Vec<u16>>>,
    /// Number of output channels (typically 2 for stereo).
    pub channels: u8,
}
