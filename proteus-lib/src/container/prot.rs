//! Container model and play settings parsing for `.prot`/`.mka`.

use matroska::{Audio, Matroska, Settings};
use rand::Rng;
use symphonia::core::audio::Channels;
use symphonia::core::sample::SampleFormat;

use log::{error, info, warn};

use crate::container::info::*;
use crate::container::play_settings::{PlaySettingsFile, PlaySettingsLegacy, SettingsTrack};
use crate::dsp::effects::convolution_reverb::{
    parse_impulse_response_spec, parse_impulse_response_tail_db, ImpulseResponseSpec,
};
use crate::dsp::effects::AudioEffect;

/// Parsed `.prot` container with resolved tracks and playback metadata.
#[derive(Debug, Clone)]
pub struct Prot {
    pub info: Info,
    file_path: Option<String>,
    file_paths: Option<Vec<Vec<String>>>,
    file_paths_dictionary: Option<Vec<String>>,
    track_ids: Option<Vec<u32>>,
    track_paths: Option<Vec<String>>,
    duration: f64,
    play_settings: Option<PlaySettingsFile>,
    impulse_response_spec: Option<ImpulseResponseSpec>,
    impulse_response_tail_db: Option<f32>,
    effects: Option<Vec<AudioEffect>>,
}

impl Prot {
    /// Load a single container file and resolve tracks.
    pub fn new(file_path: &String) -> Self {
        let info = Info::new(file_path.clone());

        println!("Info: {:?}", info);

        let mut this = Self {
            info,
            file_path: Some(file_path.clone()),
            file_paths: None,
            file_paths_dictionary: None,
            track_ids: None,
            track_paths: None,
            duration: 0.0,
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
    pub fn new_from_file_paths(file_paths: &Vec<Vec<String>>) -> Self {
        let mut file_paths_dictionary = Vec::new();
        // Add all file paths to file_paths_dictionary
        // but do not add duplicates
        for file_path in file_paths {
            for path in file_path {
                if !file_paths_dictionary.contains(path) {
                    file_paths_dictionary.push(path.clone());
                }
            }
        }

        let info = Info::new_from_file_paths(file_paths_dictionary.clone());

        let mut this = Self {
            info,
            file_path: None,
            file_paths: Some(file_paths.clone()),
            file_paths_dictionary: Some(file_paths_dictionary),
            track_ids: None,
            track_paths: None,
            duration: 0.0,
            play_settings: None,
            impulse_response_spec: None,
            impulse_response_tail_db: None,
            effects: None,
        };

        this.refresh_tracks();

        this
    }

    // fn get_duration_from_file_path(file_path: &String) -> f64 {
    //     let file = std::fs::File::open(file_path).unwrap();
    //     let symphonia: Symphonia = Symphonia::open(file).expect("Could not open file");
    // }

    /// Rebuild the active track list (e.g., after shuffle).
    pub fn refresh_tracks(&mut self) {
        let mut longest_duration = 0.0;

        if let Some(file_paths) = &self.file_paths {
            // Choose random file path from each file_paths array
            let mut track_paths: Vec<String> = Vec::new();
            for file_path in file_paths {
                let random_number = rand::thread_rng().gen_range(0..file_path.len());
                let track_path = file_path[random_number].clone();

                let index_in_dictionary = self
                    .file_paths_dictionary
                    .as_ref()
                    .unwrap()
                    .iter()
                    .position(|x| *x == track_path)
                    .unwrap();
                let duration = self.info.get_duration(index_in_dictionary as u32).unwrap();

                if duration > longest_duration {
                    longest_duration = duration;
                    self.duration = longest_duration;
                }

                track_paths.push(track_path);
            }

            self.track_paths = Some(track_paths);

            return;
        }

        if !self.file_path.is_some() {
            return;
        }

        let mut track_index_array: Vec<u32> = Vec::new();
        match self.play_settings.as_ref() {
            Some(play_settings) => match play_settings {
                PlaySettingsFile::Legacy(file) => {
                    collect_legacy_tracks(
                        file.settings.inner(),
                        &mut track_index_array,
                        &mut longest_duration,
                        &self.info,
                        &mut self.duration,
                    );
                }
                PlaySettingsFile::V1(file) => {
                    collect_tracks_from_ids(
                        &file.settings.inner().tracks,
                        &mut track_index_array,
                        &mut longest_duration,
                        &self.info,
                        &mut self.duration,
                    );
                }
                PlaySettingsFile::V2(file) => {
                    collect_tracks_from_ids(
                        &file.settings.inner().tracks,
                        &mut track_index_array,
                        &mut longest_duration,
                        &self.info,
                        &mut self.duration,
                    );
                }
                PlaySettingsFile::Unknown { .. } => {
                    error!("Unknown file format");
                }
            },
            None => {
                warn!("No play_settings.json found; no tracks resolved.");
            }
        }

        self.track_ids = Some(track_index_array);
    }

    /// Return effects parsed from play_settings, if any.
    pub fn get_effects(&self) -> Option<Vec<AudioEffect>> {
        self.effects.clone()
    }

    fn load_play_settings(&mut self) {
        println!("Loading play settings...");
        let Some(file_path) = self.file_path.as_ref() else {
            return;
        };

        let file = std::fs::File::open(file_path).unwrap();
        let mka: Matroska = Matroska::open(file).expect("Could not open file");

        let mut parsed = None;

        for attachment in &mka.attachments {
            if attachment.name == "play_settings.json" {
                match serde_json::from_slice::<PlaySettingsFile>(&attachment.data) {
                    Ok(play_settings) => {
                        parsed = Some(play_settings);
                        break;
                    }
                    Err(err) => {
                        error!("Failed to parse play_settings.json: {}", err);
                    }
                }
            }
        }

        let Some(play_settings) = parsed else {
            return;
        };

        info!("Parsed play_settings.json");

        self.impulse_response_spec = parse_impulse_response_spec(&play_settings);
        self.impulse_response_tail_db = parse_impulse_response_tail_db(&play_settings);

        match &play_settings {
            PlaySettingsFile::V1(file) => {
                self.effects = Some(file.settings.inner().effects.clone());
            }
            PlaySettingsFile::V2(file) => {
                self.effects = Some(file.settings.inner().effects.clone());
            }
            _ => {}
        }

        if let Some(effects) = self.effects.as_ref() {
            info!(
                "Loaded play_settings effects ({}): {:?}",
                effects.len(),
                effects
            );
        }

        self.play_settings = Some(play_settings);
    }

    fn get_audio_settings(file_path: &str) -> Audio {
        let file = std::fs::File::open(file_path).unwrap();

        let symph = match get_probe_result_from_string(file_path) {
            Ok(probed) => probed,
            Err(err) => {
                warn!("Failed to probe audio settings: {}", err);
                return Audio {
                    sample_rate: 0.0,
                    channels: 0,
                    bit_depth: None,
                };
            }
        };

        let first_track = match symph.format.tracks().first() {
            Some(track) => &track.codec_params,
            None => {
                warn!("No audio tracks found in {}", file_path);
                return Audio {
                    sample_rate: 0.0,
                    channels: 0,
                    bit_depth: None,
                };
            }
        };

        let channels = {
            let channels_option = first_track.channels.unwrap_or(Channels::FRONT_CENTRE);
            channels_option.iter().count()
        };

        let mut bit_depth = None;

        let bits_per_sample = first_track
            .bits_per_sample
            .or_else(|| sample_format_bits(first_track.sample_format));
        if let Some(bits) = bits_per_sample {
            bit_depth = Some(bits as u64);
        }

        let audio = Audio {
            sample_rate: first_track.sample_rate.unwrap_or(0) as f64,
            channels: channels as u64,
            bit_depth,
        };

        audio

        // let mka: Matroska = Matroska::open(file).expect("Could not open file");

        // let first_audio_settings = mka
        //     .tracks
        //     .iter()
        //     .find_map(|track| {
        //         if let Settings::Audio(audio_settings) = &track.settings {
        //             Some(audio_settings.clone()) // assuming you want to keep the settings, and they are cloneable
        //         } else {
        //             None
        //         }
        //     })
        //     .expect("Could not find audio settings");

        // first_audio_settings
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
        self.file_path.clone()
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
        // This should just be a range from 0 to the length of the track_paths or track_ids array
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
            return track_ids.into_iter().map(|id| format!("{}", id)).collect();
        }

        Vec::new()
    }

    /// Return a list of `(key, path, optional track_id)` for buffering.
    pub fn enumerated_list(&self) -> Vec<(u16, String, Option<u32>)> {
        let mut list: Vec<(u16, String, Option<u32>)> = Vec::new();
        if let Some(track_paths) = &self.track_paths {
            for (index, file_path) in track_paths.iter().enumerate() {
                list.push((index as u16, String::from(file_path), None));
            }

            return list;
        }

        if let Some(track_ids) = &self.track_ids {
            for (index, track_id) in track_ids.iter().enumerate() {
                list.push((
                    index as u16,
                    String::from(self.file_path.as_ref().unwrap()),
                    Some(*track_id),
                ));
            }

            return list;
        }

        list
    }

    /// Return container track entries for shared container streaming.
    pub fn container_track_entries(&self) -> Option<(String, Vec<(u16, u32)>)> {
        let file_path = self.file_path.as_ref()?;
        let track_ids = self.track_ids.as_ref()?;
        let mut entries = Vec::new();
        for (index, track_id) in track_ids.iter().enumerate() {
            entries.push((index as u16, *track_id));
        }
        Some((file_path.clone(), entries))
    }

    /// Get the longest selected duration (seconds).
    pub fn get_duration(&self) -> &f64 {
        &self.duration
    }

    /// Return the number of selected tracks.
    pub fn get_length(&self) -> usize {
        if let Some(file_paths) = &self.file_paths {
            return file_paths.len();
        }

        if let Some(track_ids) = &self.track_ids {
            return track_ids.len();
        }

        0
    }

    /// Return the unique file paths used for a multi-file container.
    pub fn get_file_paths_dictionary(&self) -> Vec<String> {
        match &self.file_paths_dictionary {
            Some(dictionary) => dictionary.to_vec(),
            None => Vec::new(),
        }
    }
}

fn collect_tracks_from_ids(
    tracks: &[SettingsTrack],
    track_index_array: &mut Vec<u32>,
    longest_duration: &mut f64,
    info: &Info,
    total_duration: &mut f64,
) {
    for track in tracks {
        if track.ids.is_empty() {
            continue;
        }
        let random_number = rand::thread_rng().gen_range(0..track.ids.len());
        let index = track.ids[random_number];
        if let Some(track_duration) = info.get_duration(index) {
            if track_duration > *longest_duration {
                *longest_duration = track_duration;
                *total_duration = *longest_duration;
            }
        }
        track_index_array.push(index);
    }
}

fn collect_legacy_tracks(
    settings: &PlaySettingsLegacy,
    track_index_array: &mut Vec<u32>,
    longest_duration: &mut f64,
    info: &Info,
    total_duration: &mut f64,
) {
    for track in &settings.tracks {
        let (Some(starting_index), Some(length)) = (track.starting_index, track.length) else {
            continue;
        };
        let starting_index = starting_index + 1;
        let index = rand::thread_rng().gen_range(starting_index..(starting_index + length));
        if let Some(track_duration) = info.get_duration(index) {
            if track_duration > *longest_duration {
                *longest_duration = track_duration;
                *total_duration = *longest_duration;
            }
        }
        track_index_array.push(index);
    }
}

fn sample_format_bits(sample_format: Option<SampleFormat>) -> Option<u32> {
    match sample_format {
        Some(SampleFormat::U8 | SampleFormat::S8) => Some(8),
        Some(SampleFormat::U16 | SampleFormat::S16) => Some(16),
        Some(SampleFormat::U24 | SampleFormat::S24) => Some(24),
        Some(SampleFormat::U32 | SampleFormat::S32 | SampleFormat::F32) => Some(32),
        Some(SampleFormat::F64) => Some(64),
        None => None,
    }
}
