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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_defaults_to_empty_lists() {
        let parsed: PlaySettingsV2 = serde_json::from_str("{}").unwrap();
        assert!(parsed.effects.is_empty());
        assert!(parsed.tracks.is_empty());
    }
}
