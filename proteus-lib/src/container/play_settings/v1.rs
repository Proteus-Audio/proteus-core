//! `play_settings.json` version 1 schema.

use serde::{Deserialize, Serialize};

use super::{EffectSettings, PlaySettingsContainer, SettingsTrack};

/// Top-level wrapper for V1 settings files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaySettingsV1File {
    #[serde(flatten)]
    pub settings: PlaySettingsContainer<PlaySettingsV1>,
}

/// Version 1 settings payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaySettingsV1 {
    #[serde(default)]
    pub effects: Vec<EffectSettings>,
    #[serde(default)]
    pub tracks: Vec<SettingsTrack>,
}
