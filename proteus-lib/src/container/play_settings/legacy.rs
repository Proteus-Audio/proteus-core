use serde::Deserialize;

use super::PlaySettingsContainer;

#[derive(Debug, Clone, Deserialize)]
pub struct PlaySettingsLegacyFile {
    #[serde(flatten)]
    pub settings: PlaySettingsContainer<PlaySettingsLegacy>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaySettingsLegacy {
    #[serde(default)]
    pub tracks: Vec<PlaySettingsTrackLegacy>,
    pub impulse_response: Option<String>,
    pub impulse_response_attachment: Option<String>,
    pub impulse_response_path: Option<String>,
    pub impulse_response_tail_db: Option<f32>,
    pub impulse_response_tail: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaySettingsTrackLegacy {
    #[serde(rename = "startingIndex")]
    pub starting_index: Option<u32>,
    pub length: Option<u32>,
}
