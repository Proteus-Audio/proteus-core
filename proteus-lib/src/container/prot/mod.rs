//! Container model and play settings parsing for `.prot`/`.mka`.

mod accessors;
mod helpers;
mod plan;
mod schedule;
pub mod types;

use std::panic::{catch_unwind, AssertUnwindSafe};

use log::{debug, error, info, warn};

use crate::container::info::*;
use crate::container::play_settings::{PlaySettingsFile, SettingsTrack};
use crate::container::prot_settings::{
    derive_runtime_settings, try_load_play_settings_from_container, PlaySettingsLoadError,
};
use crate::dsp::effects::convolution_reverb::ImpulseResponseSpec;
use crate::dsp::effects::AudioEffect;

pub use types::PathsTrack;
pub(crate) use types::{
    ActiveWindow, RuntimeInstanceMeta, RuntimeInstancePlan, ShuffleScheduleEntry, ShuffleSource,
};

use helpers::*;
use schedule::*;

/// Parsed `.prot` container with resolved tracks and playback metadata.
#[derive(Debug, Clone)]
pub struct Prot {
    pub info: Info,
    pub(crate) source: ProtSource,
    pub(crate) track_ids: Option<Vec<u32>>,
    pub(crate) track_paths: Option<Vec<String>>,
    pub(crate) duration: f64,
    pub(crate) shuffle_schedule: Vec<ShuffleScheduleEntry>,
    pub(crate) play_settings: Option<PlaySettingsFile>,
    pub(crate) impulse_response_spec: Option<ImpulseResponseSpec>,
    pub(crate) impulse_response_tail_db: Option<f32>,
    pub(crate) effects: Option<Vec<AudioEffect>>,
}

#[derive(Debug, Clone)]
pub(crate) enum ProtSource {
    Container {
        file_path: String,
    },
    Paths {
        file_paths: Vec<PathsTrack>,
        file_paths_dictionary: Vec<String>,
    },
}

/// Error returned when building a [`Prot`] container instance fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtError {
    Initialization(String),
}

impl std::fmt::Display for ProtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Initialization(msg) => write!(f, "prot initialization failed: {}", msg),
        }
    }
}

impl std::error::Error for ProtError {}

impl Prot {
    /// Load a single container file and resolve tracks.
    pub fn new(file_path: &str) -> Self {
        Self::try_new(file_path).unwrap_or_else(|err| panic!("Prot::new failed: {}", err))
    }

    /// Fallible constructor for a single container file.
    ///
    /// # Errors
    ///
    /// Returns [`ProtError`] when parsing or initialization panics.
    pub fn try_new(file_path: &str) -> Result<Self, ProtError> {
        catch_unwind(AssertUnwindSafe(|| Self::build_from_path(file_path))).map_err(|panic| {
            let panic_msg = panic
                .downcast_ref::<&str>()
                .map(|msg| (*msg).to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_string());
            ProtError::Initialization(panic_msg)
        })
    }

    fn build_from_path(file_path: &str) -> Self {
        let info = Info::new(file_path.to_string());

        debug!("prot info: {:?}", info);

        let mut this = Self {
            info,
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
        };

        this.load_play_settings();
        this.refresh_tracks();

        this
    }

    /// Build a container from multiple standalone file path sets.
    pub fn new_from_file_paths(file_paths: Vec<PathsTrack>) -> Self {
        let mut file_paths_dictionary = Vec::new();
        for track in file_paths.clone() {
            for path in &track.file_paths {
                if !file_paths_dictionary.contains(path) {
                    file_paths_dictionary.push(path.clone());
                }
            }
        }

        let info = Info::new_from_file_paths(file_paths_dictionary.clone());

        let mut this = Self {
            info,
            source: ProtSource::Paths {
                file_paths,
                file_paths_dictionary,
            },
            track_ids: None,
            track_paths: None,
            duration: 0.0,
            shuffle_schedule: Vec::new(),
            play_settings: None,
            impulse_response_spec: None,
            impulse_response_tail_db: None,
            effects: None,
        };

        this.refresh_tracks();

        this
    }

    /// Legacy constructor for backwards compatibility.
    pub fn new_from_file_paths_legacy(file_paths: &Vec<Vec<String>>) -> Self {
        let mut paths_track_list = Vec::new();
        for track in file_paths {
            paths_track_list.push(PathsTrack::new_from_file_paths(track.clone()));
        }
        Self::new_from_file_paths(paths_track_list)
    }

    /// Rebuild the active track list (e.g., after shuffle).
    pub fn refresh_tracks(&mut self) {
        self.track_ids = None;
        self.track_paths = None;
        self.shuffle_schedule.clear();
        self.duration = 0.0;

        if let ProtSource::Paths {
            file_paths,
            file_paths_dictionary,
        } = &self.source
        {
            let (schedule, longest_duration) =
                build_paths_shuffle_schedule(file_paths, &self.info, file_paths_dictionary);
            self.shuffle_schedule = schedule;
            self.duration = longest_duration;

            if let Some(entry) = self.shuffle_schedule.first() {
                self.track_paths = Some(sources_to_track_paths(&entry.sources));
            }

            return;
        }

        match self.play_settings.as_ref() {
            Some(play_settings) => match play_settings {
                PlaySettingsFile::Legacy(file) => {
                    let mut longest_duration = 0.0;
                    let mut track_index_array: Vec<u32> = Vec::new();
                    collect_legacy_tracks(
                        file.settings.inner(),
                        &mut track_index_array,
                        &mut longest_duration,
                        &self.info,
                        &mut self.duration,
                    );
                    self.track_ids = Some(track_index_array.clone());
                    self.shuffle_schedule = vec![ShuffleScheduleEntry {
                        at_ms: 0,
                        sources: track_index_array
                            .into_iter()
                            .map(ShuffleSource::TrackId)
                            .collect(),
                    }];
                }
                PlaySettingsFile::Unknown { .. } => {
                    error!("unknown file format");
                }
                _ => {
                    if let Some(tracks) = versioned_tracks(play_settings) {
                        let (schedule, longest_duration) =
                            build_id_shuffle_schedule(tracks, &self.info);
                        self.shuffle_schedule = schedule;
                        self.duration = longest_duration;
                    }
                }
            },
            None => {
                warn!("no play_settings.json found; no tracks resolved");
            }
        }

        if let Some(entry) = self.shuffle_schedule.first() {
            self.track_ids = Some(sources_to_track_ids(&entry.sources));
        }
    }

    fn load_play_settings(&mut self) {
        let ProtSource::Container { file_path } = &self.source else {
            return;
        };

        let play_settings = match try_load_play_settings_from_container(file_path) {
            Ok(play_settings) => play_settings,
            Err(PlaySettingsLoadError::MissingAttachment) => return,
            Err(err) => {
                warn!("unable to load play_settings.json: {}", err);
                return;
            }
        };

        let runtime = derive_runtime_settings(&play_settings);
        self.impulse_response_spec = runtime.impulse_response_spec;
        self.impulse_response_tail_db = runtime.impulse_response_tail_db;
        self.effects = runtime.effects;

        if let Some(effects) = self.effects.as_ref() {
            info!(
                "Loaded play_settings effects ({}): {:?}",
                effects.len(),
                effects
            );
        }

        self.play_settings = Some(play_settings);
    }
}

fn versioned_tracks(play_settings: &PlaySettingsFile) -> Option<&[SettingsTrack]> {
    play_settings
        .versioned_payload()
        .map(|payload| payload.tracks.as_slice())
}

fn versioned_tracks_mut(play_settings: &mut PlaySettingsFile) -> Option<&mut Vec<SettingsTrack>> {
    play_settings
        .versioned_payload_mut()
        .map(|payload| &mut payload.tracks)
}

#[cfg(test)]
mod tests;
