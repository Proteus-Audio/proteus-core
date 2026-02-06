use serde::Deserialize;

use super::PlaySettingsContainer;

#[derive(Debug, Clone, Deserialize)]
pub struct PlaySettingsV1File {
    #[serde(flatten)]
    pub settings: PlaySettingsContainer<PlaySettingsV1>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaySettingsV1 {
    #[serde(default)]
    pub tracks: Vec<PlaySettingsTrackV1>,
    pub impulse_response: Option<String>,
    pub impulse_response_attachment: Option<String>,
    pub impulse_response_path: Option<String>,
    pub impulse_response_tail_db: Option<f32>,
    pub impulse_response_tail: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaySettingsTrackV1 {
    #[serde(default)]
    pub ids: Vec<u32>,
}
