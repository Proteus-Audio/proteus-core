//! `play_settings.json` version 2 schema.

use serde::{Deserialize, Serialize};

use super::{EffectSettings, PlaySettingsContainer, SettingsTrack};

/// Top-level wrapper for V2 settings files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaySettingsV2File {
    #[serde(flatten)]
    pub settings: PlaySettingsContainer<PlaySettingsV2>,
}

/// Version 2 settings payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaySettingsV2 {
    #[serde(default)]
    pub effects: Vec<EffectSettings>,
    #[serde(default)]
    pub tracks: Vec<SettingsTrack>,
}
