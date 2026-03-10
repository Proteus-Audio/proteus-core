//! Player construction helpers.

use rodio::Sink;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::{Arc, Mutex};

use super::{
    default_output_stream_handle, Player, PlayerInitError, PlayerInitOptions, PlayerSource,
    PlayerState, OUTPUT_METER_REFRESH_HZ,
};
use crate::container::info::Info;
use crate::container::prot::{PathsTrack, Prot};
use crate::playback::engine::{DspChainMetrics, PlaybackBufferSettings};
use crate::playback::output_meter::OutputMeter;

impl Player {
    /// Fallible constructor from a typed source and options.
    ///
    /// # Errors
    ///
    /// Returns [`PlayerInitError::ProtInitialization`] when opening/parsing a
    /// container path fails.
    pub fn try_from_source_with_options(
        source: PlayerSource,
        options: PlayerInitOptions,
    ) -> Result<Self, PlayerInitError> {
        let (prot, info) = load_player_source(source)?;
        let sink = create_player_sink();
        let channels = info.channels as usize;
        let sample_rate = info.sample_rate;
        let effects = load_initial_effects(&prot);

        let mut player = Self {
            info,
            finished_tracks: Arc::new(Mutex::new(Vec::new())),
            state: Arc::new(Mutex::new(PlayerState::Stopped)),
            abort: Arc::new(AtomicBool::new(false)),
            ts: Arc::new(Mutex::new(0.0)),
            playback_thread_exists: Arc::new(AtomicBool::new(true)),
            playback_thread_handle: Arc::new(Mutex::new(None)),
            playback_id: Arc::new(AtomicU64::new(0)),
            duration: Arc::new(Mutex::new(0.0)),
            prot,
            audio_heard: Arc::new(AtomicBool::new(false)),
            play_command_ms: Arc::new(AtomicU64::new(0)),
            volume: Arc::new(Mutex::new(0.8)),
            sink,
            output_stream: default_output_stream_handle(),
            reporter: None,
            buffer_settings: Arc::new(Mutex::new(PlaybackBufferSettings::new(20.0))),
            effects,
            inline_effects_update: Arc::new(Mutex::new(None)),
            inline_track_mix_updates: Arc::new(Mutex::new(Vec::new())),
            dsp_metrics: Arc::new(Mutex::new(DspChainMetrics::default())),
            effects_reset: Arc::new(AtomicU64::new(0)),
            output_meter: Arc::new(Mutex::new(OutputMeter::new(
                channels,
                sample_rate,
                OUTPUT_METER_REFRESH_HZ,
            ))),
            buffering_done: Arc::new(AtomicBool::new(false)),
            last_chunk_ms: Arc::new(AtomicU64::new(0)),
            last_time_update_ms: Arc::new(AtomicU64::new(0)),
            next_resume_fade_ms: Arc::new(Mutex::new(None)),
            end_of_stream_action: Arc::new(Mutex::new(options.end_of_stream_action)),
            handle_count: Arc::new(AtomicUsize::new(1)),
            shutdown_once: Arc::new(AtomicBool::new(false)),
            impulse_response_override: None,
            impulse_response_tail_override: None,
        };

        player.initialize_thread(None);

        Ok(player)
    }

    /// Create a new player for a single container path.
    ///
    /// # Arguments
    ///
    /// * `file_path` - Path to a `.prot`/`.mka` container file.
    ///
    /// # Panics
    ///
    /// Panics if the container cannot be opened or parsed. Prefer
    /// [`Self::try_from_source_with_options`] for fallible construction.
    pub fn new(file_path: &str) -> Self {
        Self::from_source(PlayerSource::ContainerPath(file_path.to_string()))
    }

    /// Create a new player for a single container path with explicit options.
    ///
    /// # Arguments
    ///
    /// * `file_path` - Path to a `.prot`/`.mka` container file.
    /// * `options` - Player initialization options.
    ///
    /// # Panics
    ///
    /// Panics if the container cannot be opened or parsed. Prefer
    /// [`Self::try_from_source_with_options`] for fallible construction.
    pub fn new_with_options(file_path: &str, options: PlayerInitOptions) -> Self {
        Self::from_source_with_options(PlayerSource::ContainerPath(file_path.to_string()), options)
    }

    /// Create a new player for a set of standalone file paths.
    ///
    /// # Arguments
    ///
    /// * `file_paths` - Pre-normalized track path groups.
    ///
    /// # Panics
    ///
    /// Panics if runtime initialization fails.
    pub fn new_from_file_paths(file_paths: Vec<PathsTrack>) -> Self {
        Self::from_source(PlayerSource::FilePaths(file_paths))
    }

    /// Create a new player for standalone file paths with explicit options.
    ///
    /// # Arguments
    ///
    /// * `file_paths` - Pre-normalized track path groups.
    /// * `options` - Player initialization options.
    ///
    /// # Panics
    ///
    /// Panics if runtime initialization fails.
    pub fn new_from_file_paths_with_options(
        file_paths: Vec<PathsTrack>,
        options: PlayerInitOptions,
    ) -> Self {
        Self::from_source_with_options(PlayerSource::FilePaths(file_paths), options)
    }

    /// Create a new player for legacy standalone file-path groups.
    ///
    /// # Arguments
    ///
    /// * `file_paths` - Legacy track grouping shape where each inner vector is
    ///   interpreted as one track candidate set.
    ///
    /// # Panics
    ///
    /// Panics if runtime initialization fails.
    pub fn new_from_file_paths_legacy(file_paths: Vec<Vec<String>>) -> Self {
        Self::new_from_file_paths_legacy_with_options(file_paths, PlayerInitOptions::default())
    }

    /// Create a new player for legacy standalone file-path groups with options.
    ///
    /// # Arguments
    ///
    /// * `file_paths` - Legacy track grouping shape where each inner vector is
    ///   interpreted as one track candidate set.
    /// * `options` - Player initialization options.
    ///
    /// # Panics
    ///
    /// Panics if runtime initialization fails.
    pub fn new_from_file_paths_legacy_with_options(
        file_paths: Vec<Vec<String>>,
        options: PlayerInitOptions,
    ) -> Self {
        let tracks = file_paths
            .into_iter()
            .map(PathsTrack::new_from_file_paths)
            .collect();
        Self::from_source_with_options(PlayerSource::FilePaths(tracks), options)
    }

    /// Create a player from either a container path or standalone file paths.
    ///
    /// Exactly one input source is required. Passing both `path` and `paths`
    /// is rejected as [`PlayerInitError::AmbiguousSource`].
    ///
    /// # Arguments
    ///
    /// * `path` - Optional container path.
    /// * `paths` - Optional standalone track path groups.
    ///
    /// # Panics
    ///
    /// Panics when source selection is invalid or initialization fails.
    /// Prefer [`Self::try_new_from_path_or_paths_with_options`] for fallible
    /// construction.
    pub fn new_from_path_or_paths(path: Option<&str>, paths: Option<Vec<PathsTrack>>) -> Self {
        Self::new_from_path_or_paths_with_options(path, paths, PlayerInitOptions::default())
    }

    /// Create a player from either a container path or standalone file paths
    /// with explicit initialization options.
    ///
    /// For a fallible constructor that returns a typed error when no source is
    /// provided, see [`Self::try_new_from_path_or_paths_with_options`].
    ///
    /// # Arguments
    ///
    /// * `path` - Optional container path.
    /// * `paths` - Optional standalone track path groups.
    /// * `options` - Player initialization options.
    ///
    /// # Panics
    ///
    /// Panics when source selection is invalid or initialization fails.
    /// Prefer [`Self::try_new_from_path_or_paths_with_options`] for fallible
    /// construction.
    pub fn new_from_path_or_paths_with_options(
        path: Option<&str>,
        paths: Option<Vec<PathsTrack>>,
        options: PlayerInitOptions,
    ) -> Self {
        Self::try_new_from_path_or_paths_with_options(path, paths, options).unwrap_or_else(|err| {
            panic!("Player initialization failed: {}", err);
        })
    }

    /// Fallible constructor from optional container/file-path inputs.
    ///
    /// # Arguments
    ///
    /// * `path` - Optional container path.
    /// * `paths` - Optional standalone track path groups.
    /// * `options` - Player initialization options.
    ///
    /// # Errors
    ///
    /// Returns [`PlayerInitError::MissingSource`] when both `path` and `paths`
    /// are missing, or [`PlayerInitError::AmbiguousSource`] when both are
    /// provided.
    pub fn try_new_from_path_or_paths_with_options(
        path: Option<&str>,
        paths: Option<Vec<PathsTrack>>,
        options: PlayerInitOptions,
    ) -> Result<Self, PlayerInitError> {
        match (path, paths) {
            (Some(path), None) => Self::try_from_source_with_options(
                PlayerSource::ContainerPath(path.to_string()),
                options,
            ),
            (None, Some(file_paths)) => {
                Self::try_from_source_with_options(PlayerSource::FilePaths(file_paths), options)
            }
            (None, None) => Err(PlayerInitError::MissingSource),
            (Some(_), Some(_)) => Err(PlayerInitError::AmbiguousSource),
        }
    }
}

fn load_player_source(source: PlayerSource) -> Result<(Arc<Mutex<Prot>>, Info), PlayerInitError> {
    match source {
        PlayerSource::ContainerPath(path) => {
            let prot = Arc::new(Mutex::new(
                Prot::try_new(&path).map_err(PlayerInitError::ProtInitialization)?,
            ));
            let info = prot.lock().unwrap().info.clone();
            Ok((prot, info))
        }
        PlayerSource::FilePaths(paths) => {
            let prot = Arc::new(Mutex::new(Prot::new_from_file_paths(paths)));
            let info = Info::new_from_file_paths(prot.lock().unwrap().get_file_paths_dictionary());
            Ok((prot, info))
        }
    }
}

fn create_player_sink() -> Arc<Mutex<Sink>> {
    let (sink, _queue) = Sink::new();
    Arc::new(Mutex::new(sink))
}

fn load_initial_effects(
    prot: &Arc<Mutex<Prot>>,
) -> Arc<Mutex<Vec<crate::dsp::effects::AudioEffect>>> {
    let effects = prot.lock().unwrap().get_effects().unwrap_or_default();
    Arc::new(Mutex::new(effects))
}

#[cfg(test)]
mod tests {
    use crate::container::prot::PathsTrack;

    use super::super::{Player, PlayerInitError, PlayerInitOptions};

    #[test]
    fn player_init_error_display_is_actionable() {
        assert_eq!(
            PlayerInitError::MissingSource.to_string(),
            "player source input is required"
        );
        assert_eq!(
            PlayerInitError::AmbiguousSource.to_string(),
            "player source input must be exactly one of path or file paths"
        );
    }

    #[test]
    fn try_new_from_path_or_paths_requires_input_source() {
        let result = Player::try_new_from_path_or_paths_with_options(
            None,
            None,
            PlayerInitOptions::default(),
        );
        assert!(matches!(result, Err(PlayerInitError::MissingSource)));
    }

    #[test]
    fn try_new_from_path_or_paths_rejects_ambiguous_inputs() {
        let result = Player::try_new_from_path_or_paths_with_options(
            Some("/tmp/example.prot"),
            Some(vec![PathsTrack::new_from_file_paths(vec![
                "/tmp/a.wav".to_string()
            ])]),
            PlayerInitOptions::default(),
        );
        assert!(matches!(result, Err(PlayerInitError::AmbiguousSource)));
    }
}
