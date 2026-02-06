//! Legacy `play_settings.json` schema (pre-versioned).

use serde::Deserialize;

use super::PlaySettingsContainer;

/// Top-level wrapper for legacy settings files.
#[derive(Debug, Clone, Deserialize)]
pub struct PlaySettingsLegacyFile {
    #[serde(flatten)]
    pub settings: PlaySettingsContainer<PlaySettingsLegacy>,
}

/// Legacy settings payload (tracks only).
#[derive(Debug, Clone, Deserialize)]
pub struct PlaySettingsLegacy {
    #[serde(default)]
    pub tracks: Vec<PlaySettingsTrackLegacy>,
}

/// Legacy per-track settings.
#[derive(Debug, Clone, Deserialize)]
pub struct PlaySettingsTrackLegacy {
    #[serde(rename = "startingIndex")]
    pub starting_index: Option<u32>,
    pub length: Option<u32>,
}
