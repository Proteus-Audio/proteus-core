//! Legacy `play_settings.json` schema (pre-versioned).

use serde::{Deserialize, Serialize};

use super::PlaySettingsContainer;

/// Top-level wrapper for legacy settings files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PlaySettingsLegacyFile {
    /// The legacy settings payload, which may be nested or flat.
    #[serde(flatten)]
    pub settings: PlaySettingsContainer<PlaySettingsLegacy>,
}

/// Legacy settings payload (tracks only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PlaySettingsLegacy {
    /// Per-track legacy settings read from the container.
    #[serde(default)]
    pub tracks: Vec<PlaySettingsTrackLegacy>,
}

/// Legacy per-track settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PlaySettingsTrackLegacy {
    /// Index of the first take to use for this track.
    #[serde(rename = "startingIndex")]
    pub starting_index: Option<u32>,
    /// Number of takes available for this track.
    pub length: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_track_uses_starting_index_alias() {
        let parsed: PlaySettingsTrackLegacy =
            serde_json::from_str(r#"{"startingIndex": 3, "length": 2}"#).unwrap();
        assert_eq!(parsed.starting_index, Some(3));
        assert_eq!(parsed.length, Some(2));
    }
}
