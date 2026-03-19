//! Shared types for the prot module.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ShuffleSource {
    TrackId(u32),
    FilePath(String),
}

#[derive(Debug, Clone)]
pub(crate) struct ShuffleScheduleEntry {
    pub at_ms: u64,
    pub sources: Vec<ShuffleSource>,
}

/// Active time range for one instance in milliseconds relative to playback start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveWindow {
    pub start_ms: u64,
    pub end_ms: Option<u64>,
}

/// Runtime metadata for one concrete source instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeInstanceMeta {
    pub instance_id: usize,
    pub logical_track_index: usize,
    pub slot_index: usize,
    pub source_key: ShuffleSource,
    pub active_windows: Vec<ActiveWindow>,
    pub selection_index: usize,
    pub occurrence_index: usize,
}

/// Expanded runtime plan used by schedule-driven routing/mixing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeInstancePlan {
    pub logical_track_count: usize,
    pub instances: Vec<RuntimeInstanceMeta>,
    pub event_boundaries_ms: Vec<u64>,
}

/// Standalone file-path track configuration.
#[derive(Debug, Clone)]
pub struct PathsTrack {
    /// Candidate file paths for this track.
    pub file_paths: Vec<String>,
    /// Track gain scalar.
    pub level: f32,
    /// Track pan position.
    pub pan: f32,
    /// Number of selections to pick per refresh.
    pub selections_count: u32,
    /// Timestamps where this track is reshuffled.
    pub shuffle_points: Vec<String>,
}

impl PathsTrack {
    /// Create a new PathsTrack from a vector of file paths.
    pub fn new_from_file_paths(file_paths: Vec<String>) -> Self {
        PathsTrack {
            file_paths,
            level: 1.0,
            pan: 0.0,
            selections_count: 1,
            shuffle_points: Vec::new(),
        }
    }
}

/// Slot identity within the schedule layout.
pub(super) struct SlotPlacement {
    pub slot_index: usize,
    pub logical_track_index: usize,
    pub selection_index: usize,
}

/// Relative segment time range in milliseconds.
pub(super) struct SegmentRange {
    pub start_ms: u64,
    pub end_ms: Option<u64>,
}
