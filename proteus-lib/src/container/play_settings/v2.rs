use serde::Deserialize;

use super::PlaySettingsContainer;

#[derive(Debug, Clone, Deserialize)]
pub struct PlaySettingsV2File {
    #[serde(flatten)]
    pub settings: PlaySettingsContainer<PlaySettingsV2>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaySettingsV2 {
    #[serde(default)]
    pub tracks: Vec<PlaySettingsTrackV2>,
    pub impulse_response: Option<String>,
    pub impulse_response_attachment: Option<String>,
    pub impulse_response_path: Option<String>,
    pub impulse_response_tail_db: Option<f32>,
    pub impulse_response_tail: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaySettingsTrackV2 {
    #[serde(default)]
    pub ids: Vec<u32>,
}
