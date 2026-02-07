//! Legacy `play_settings.json` schema (pre-versioned).

use serde::{Deserialize, Serialize};

use super::PlaySettingsContainer;

/// Top-level wrapper for legacy settings files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaySettingsLegacyFile {
    #[serde(flatten)]
    pub settings: PlaySettingsContainer<PlaySettingsLegacy>,
}

/// Legacy settings payload (tracks only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaySettingsLegacy {
    #[serde(default)]
    pub tracks: Vec<PlaySettingsTrackLegacy>,
}

/// Legacy per-track settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaySettingsTrackLegacy {
    #[serde(rename = "startingIndex")]
    pub starting_index: Option<u32>,
    pub length: Option<u32>,
}
