//! Accessors and view helpers for [`Prot`].

use log::warn;

use crate::dsp::effects::convolution_reverb::ImpulseResponseSpec;
use crate::dsp::effects::AudioEffect;

use super::{Prot, ProtSource};

impl Prot {
    /// Return effects parsed from play_settings, if any.
    pub fn get_effects(&self) -> Option<Vec<AudioEffect>> {
        self.effects.clone()
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
