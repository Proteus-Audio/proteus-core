use serde::Deserialize;

use super::{EffectSettings, PlaySettingsContainer, SettingsTrack};

#[derive(Debug, Clone, Deserialize)]
pub struct PlaySettingsV1File {
    #[serde(flatten)]
    pub settings: PlaySettingsContainer<PlaySettingsV1>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaySettingsV1 {
    #[serde(default)]
    pub effects: Vec<EffectSettings>,
    #[serde(default)]
    pub tracks: Vec<SettingsTrack>,
}
