//! Container model and play settings parsing for `.prot`/`.mka`.

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
                    error!("Unknown file format");
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
                warn!("No play_settings.json found; no tracks resolved.");
            }
        }

        if let Some(entry) = self.shuffle_schedule.first() {
            self.track_ids = Some(sources_to_track_ids(&entry.sources));
        }
    }

    /// Return effects parsed from play_settings, if any.
    pub fn get_effects(&self) -> Option<Vec<AudioEffect>> {
        self.effects.clone()
    }

    fn load_play_settings(&mut self) {
        let ProtSource::Container { file_path } = &self.source else {
            return;
        };

        let play_settings = match try_load_play_settings_from_container(file_path) {
            Ok(play_settings) => play_settings,
            Err(PlaySettingsLoadError::MissingAttachment) => return,
            Err(err) => {
                warn!("Unable to load play_settings.json: {}", err);
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

    /// Get the convolution impulse response spec, if configured.
    pub fn get_impulse_response_spec(&self) -> Option<ImpulseResponseSpec> {
        self.impulse_response_spec.clone()
    }

    /// Get the configured impulse response tail trim in dB, if any.
    pub fn get_impulse_response_tail_db(&self) -> Option<f32> {
        self.impulse_response_tail_db
    }

    /// Return the container path if this is a `.prot`/`.mka` file.
    pub fn get_container_path(&self) -> Option<String> {
        match &self.source {
            ProtSource::Container { file_path } => Some(file_path.clone()),
            ProtSource::Paths { .. } => None,
        }
    }

    /// Override the impulse response spec at runtime.
    pub fn set_impulse_response_spec(&mut self, spec: ImpulseResponseSpec) {
        self.impulse_response_spec = Some(spec);
    }

    /// Override the impulse response tail trim at runtime.
    pub fn set_impulse_response_tail_db(&mut self, tail_db: f32) {
        self.impulse_response_tail_db = Some(tail_db);
    }

    /// Return per-track keys for UI selection.
    pub fn get_keys(&self) -> Vec<u32> {
        if let Some(track_paths) = &self.track_paths {
            return (0..track_paths.len() as u32).collect();
        }

        if let Some(track_ids) = &self.track_ids {
            return (0..track_ids.len() as u32).collect();
        }

        Vec::new()
    }

    /// Return per-track identifiers or file paths for display.
    pub fn get_ids(&self) -> Vec<String> {
        if let Some(track_paths) = &self.track_paths {
            return track_paths.clone();
        }

        if let Some(track_ids) = &self.track_ids {
            return track_ids.iter().map(|id| format!("{}", id)).collect();
        }

        Vec::new()
    }

    /// Return a list of `(key, path, optional track_id)` for buffering.
    pub fn enumerated_list(&self) -> Vec<(u16, String, Option<u32>)> {
        let mut list: Vec<(u16, String, Option<u32>)> = Vec::new();
        if let Some(track_paths) = &self.track_paths {
            for (index, file_path) in track_paths.iter().enumerate() {
                let Ok(key) = u16::try_from(index) else {
                    warn!("skipping track index {} that exceeds u16 key range", index);
                    continue;
                };
                list.push((key, String::from(file_path), None));
            }

            return list;
        }

        if let (Some(track_ids), ProtSource::Container { file_path }) =
            (&self.track_ids, &self.source)
        {
            for (index, track_id) in track_ids.iter().enumerate() {
                let Ok(key) = u16::try_from(index) else {
                    warn!("skipping track index {} that exceeds u16 key range", index);
                    continue;
                };
                list.push((key, file_path.clone(), Some(*track_id)));
            }

            return list;
        }

        list
    }

    /// Return container track entries for shared container streaming.
    pub fn container_track_entries(&self) -> Option<(String, Vec<(u16, u32)>)> {
        let file_path = match &self.source {
            ProtSource::Container { file_path } => file_path,
            ProtSource::Paths { .. } => return None,
        };
        let track_ids = self.track_ids.as_ref()?;
        let mut entries = Vec::new();
        for (index, track_id) in track_ids.iter().enumerate() {
            let Ok(key) = u16::try_from(index) else {
                warn!("skipping track index {} that exceeds u16 key range", index);
                continue;
            };
            entries.push((key, *track_id));
        }
        Some((file_path.clone(), entries))
    }

    /// Get the longest selected duration (seconds).
    pub fn get_duration(&self) -> &f64 {
        &self.duration
    }

    /// Return the number of selected tracks.
    pub fn get_length(&self) -> usize {
        if let Some(track_paths) = &self.track_paths {
            return track_paths.len();
        }

        if let ProtSource::Paths { file_paths, .. } = &self.source {
            return file_paths.len();
        }

        if let Some(track_ids) = &self.track_ids {
            return track_ids.len();
        }

        0
    }

    /// Return the unique file paths used for a multi-file container.
    pub fn get_file_paths_dictionary(&self) -> Vec<String> {
        match &self.source {
            ProtSource::Paths {
                file_paths_dictionary,
                ..
            } => file_paths_dictionary.clone(),
            ProtSource::Container { .. } => Vec::new(),
        }
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
