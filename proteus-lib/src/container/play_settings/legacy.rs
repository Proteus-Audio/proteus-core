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
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaySettingsTrackLegacy {
    #[serde(rename = "startingIndex")]
    pub starting_index: Option<u32>,
    pub length: Option<u32>,
}
