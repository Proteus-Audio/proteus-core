use serde::Deserialize;

use super::{EffectSettings, PlaySettingsContainer, SettingsTrack};

#[derive(Debug, Clone, Deserialize)]
pub struct PlaySettingsV2File {
    #[serde(flatten)]
    pub settings: PlaySettingsContainer<PlaySettingsV2>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaySettingsV2 {
    #[serde(default)]
    pub effects: Vec<EffectSettings>,
    #[serde(default)]
    pub tracks: Vec<SettingsTrack>,
}
