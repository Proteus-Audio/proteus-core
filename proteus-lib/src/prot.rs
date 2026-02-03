use matroska::{Audio, Matroska, Settings};
use rand::Rng;
use symphonia::core::audio::Channels;

use crate::info::*;

#[derive(Debug, Clone)]
pub struct Prot {
    pub info: Info,
    file_path: Option<String>,
    file_paths: Option<Vec<Vec<String>>>,
    file_paths_dictionary: Option<Vec<String>>,
    track_ids: Option<Vec<u32>>,
    track_paths: Option<Vec<String>>,
    duration: f64,
}

impl Prot {
    pub fn new(file_path: &String) -> Self {
        let info = Info::new(file_path.clone());

        let mut this = Self {
            info,
            file_path: Some(file_path.clone()),
            file_paths: None,
            file_paths_dictionary: None,
            track_ids: None,
            track_paths: None,
            duration: 0.0,
        };

        this.refresh_tracks();

        this
    }

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
        };

        this.refresh_tracks();

        this
    }

    // fn get_duration_from_file_path(file_path: &String) -> f64 {
    //     let file = std::fs::File::open(file_path).unwrap();
    //     let symphonia: Symphonia = Symphonia::open(file).expect("Could not open file");
    // }

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

        let file_path = self.file_path.as_ref().unwrap();
        let file = std::fs::File::open(file_path).unwrap();

        let mka: Matroska = Matroska::open(file).expect("Could not open file");

        let mut track_index_array: Vec<u32> = Vec::new();
        mka.attachments.iter().for_each(|attachment| {
            // Only print if name is "play_settings.json"
            if attachment.name == "play_settings.json" {
                // read json data from attachment.data to object
                let json_data: serde_json::Value =
                    serde_json::from_slice(&attachment.data).unwrap();

                let encoder_version = json_data["encoder_version"].as_f64();

                // For each track in json_data, print the track number
                json_data["play_settings"]["tracks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .for_each(|track| {
                        if let Some(_version) = encoder_version {
                            let indexes = track["ids"].as_array().unwrap();
                            if indexes.len() == 0 {
                                return;
                            }
                            let random_number = rand::thread_rng().gen_range(0..indexes.len());
                            let index = indexes[random_number].to_string().parse::<u32>().unwrap();
                            if let Some(track_duration) = self.info.get_duration(index) {
                                if track_duration > longest_duration {
                                    longest_duration = track_duration;
                                    self.duration = longest_duration;
                                }
                            }
                            track_index_array.push(index);
                        } else {
                            let starting_index =
                                track["startingIndex"].to_string().parse::<u32>().unwrap() + 1;
                            let length = track["length"].to_string().parse::<u32>().unwrap();

                            // Get random number between starting_index and starting_index + length
                            let index = rand::thread_rng()
                                .gen_range(starting_index..(starting_index + length));

                            if let Some(track_duration) = self.info.get_duration(index) {
                                if track_duration > longest_duration {
                                    longest_duration = track_duration;
                                    self.duration = longest_duration;
                                }
                            }

                            track_index_array.push(index);
                        }
                    });
            }
        });

        self.track_ids = Some(track_index_array);
    }

    fn get_audio_settings(file_path: &str) -> Audio {
        let file = std::fs::File::open(file_path).unwrap();

        let symph = get_probe_result_from_string(file_path);

        symph.format.tracks();

        let first_track = &symph.format.tracks().first().unwrap().codec_params;

        let channels = {
            let channels_option = first_track.channels.unwrap_or(Channels::FRONT_CENTRE);
            channels_option.iter().count()
        };

        let mut bit_depth = None;

        if let Some(bits) = first_track.bits_per_sample {
            bit_depth = Some(bits as u64)
        }

        let audio = Audio {
            sample_rate: first_track.sample_rate.unwrap() as f64,
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

    pub fn get_ids(&self) -> Vec<String> {
        if let Some(track_paths) = &self.track_paths {
            return track_paths.clone();
        }

        if let Some(track_ids) = &self.track_ids {
            return track_ids.into_iter().map(|id| format!("{}", id)).collect();
        }

        Vec::new()
    }

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

    pub fn get_duration(&self) -> &f64 {
        &self.duration
    }

    pub fn get_length(&self) -> usize {
        if let Some(file_paths) = &self.file_paths {
            return file_paths.len();
        }

        if let Some(track_ids) = &self.track_ids {
            return track_ids.len();
        }

        0
    }

    pub fn get_file_paths_dictionary(&self) -> Vec<String> {
        match &self.file_paths_dictionary {
            Some(dictionary) => dictionary.to_vec(),
            None => Vec::new(),
        }
    }
}
