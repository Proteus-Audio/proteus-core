//! `play_settings.json` version 3 schema.

use serde::{Deserialize, Serialize};

use super::{EffectSettings, PlaySettingsContainer, SettingsTrack};

/// Top-level wrapper for V3 settings files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaySettingsV3File {
    #[serde(flatten)]
    pub settings: PlaySettingsContainer<PlaySettingsV3>,
}

/// Version 3 settings payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaySettingsV3 {
    #[serde(default)]
    pub effects: Vec<EffectSettings>,
    #[serde(default)]
    pub tracks: Vec<SettingsTrack>,
}
