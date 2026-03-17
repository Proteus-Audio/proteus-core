//! Serde models for `play_settings.json` with versioned decoding.

use log::{info, warn};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub(crate) mod legacy;

pub(crate) use legacy::{PlaySettingsLegacy, PlaySettingsLegacyFile};

/// Effect settings variants that can appear in the settings file.
pub(crate) type EffectSettings = serde_json::Value;

/// Track-level configuration shared by newer settings versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SettingsTrack {
    /// Playback volume level for this track (linear gain, 1.0 = unity).
    pub level: f32,
    /// Stereo pan position for this track (−1.0 = full left, +1.0 = full right).
    pub pan: f32,
    /// Ordered list of audio take IDs available for random selection on this track.
    pub ids: Vec<u32>,
    /// Human-readable display name for this track.
    pub name: String,
    /// Filesystem-safe version of the track name, used for file and key lookups.
    pub safe_name: String,
    /// Number of takes to select simultaneously when building a playback plan.
    #[serde(default = "default_selections_count")]
    pub selections_count: u32,
    /// Named shuffle points at which the track may rotate to the next selection.
    #[serde(default)]
    pub shuffle_points: Vec<String>,
}

/// Shared payload used by versioned `play_settings.json` schemas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PlaySettingsPayload {
    /// DSP effect chain applied to the final mix, in processing order.
    #[serde(default)]
    pub effects: Vec<EffectSettings>,
    /// Per-track volume, pan, and selection configuration.
    #[serde(default)]
    pub tracks: Vec<SettingsTrack>,
}

/// Top-level wrapper shared by versioned settings files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct VersionedPlaySettingsFile<T> {
    /// The settings payload, which may be nested under a `play_settings` key or flat.
    #[serde(flatten)]
    pub settings: PlaySettingsContainer<T>,
}

/// Version 1 settings payload.
pub(crate) type PlaySettingsV1 = PlaySettingsPayload;
/// Top-level wrapper for V1 settings files.
pub(crate) type PlaySettingsV1File = VersionedPlaySettingsFile<PlaySettingsV1>;
/// Version 2 settings payload.
pub(crate) type PlaySettingsV2 = PlaySettingsPayload;
/// Top-level wrapper for V2 settings files.
pub(crate) type PlaySettingsV2File = VersionedPlaySettingsFile<PlaySettingsV2>;
/// Version 3 settings payload.
pub(crate) type PlaySettingsV3 = PlaySettingsPayload;
/// Top-level wrapper for V3 settings files.
pub(crate) type PlaySettingsV3File = VersionedPlaySettingsFile<PlaySettingsV3>;

fn default_selections_count() -> u32 {
    1
}

/// Wrapper allowing `play_settings` to be nested or flat.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum PlaySettingsContainer<T> {
    /// Settings wrapped under a `play_settings` key in the JSON object.
    Nested {
        /// The wrapped settings payload.
        play_settings: T,
    },
    /// Settings present directly at the root of the JSON object.
    Flat(T),
}

impl<T> PlaySettingsContainer<T> {
    /// Return the inner settings payload, regardless of nesting.
    pub fn inner(&self) -> &T {
        match self {
            PlaySettingsContainer::Nested { play_settings } => play_settings,
            PlaySettingsContainer::Flat(inner) => inner,
        }
    }

    /// Return mutable access to the inner settings payload.
    pub fn inner_mut(&mut self) -> &mut T {
        match self {
            PlaySettingsContainer::Nested { play_settings } => play_settings,
            PlaySettingsContainer::Flat(inner) => inner,
        }
    }
}

/// Versioned settings file representation.
#[derive(Debug, Clone)]
pub(crate) enum PlaySettingsFile {
    /// Legacy (pre-versioned) settings format without an `encoder_version` field.
    Legacy(PlaySettingsLegacyFile),
    /// Version 1 settings format.
    V1(PlaySettingsV1File),
    /// Version 2 settings format.
    V2(PlaySettingsV2File),
    /// Version 3 settings format.
    V3(PlaySettingsV3File),
    /// Settings with an unrecognized `encoder_version`; raw JSON is preserved.
    Unknown {
        /// The raw JSON value preserved for round-trip serialization.
        raw: serde_json::Value,
    },
}

impl PlaySettingsFile {
    /// Return normalized modern payload for V1/V2/V3 settings.
    pub(crate) fn versioned_payload(&self) -> Option<&PlaySettingsPayload> {
        match self {
            PlaySettingsFile::V1(file) => Some(file.settings.inner()),
            PlaySettingsFile::V2(file) => Some(file.settings.inner()),
            PlaySettingsFile::V3(file) => Some(file.settings.inner()),
            _ => None,
        }
    }

    /// Return mutable normalized modern payload for V1/V2/V3 settings.
    pub(crate) fn versioned_payload_mut(&mut self) -> Option<&mut PlaySettingsPayload> {
        match self {
            PlaySettingsFile::V1(file) => Some(file.settings.inner_mut()),
            PlaySettingsFile::V2(file) => Some(file.settings.inner_mut()),
            PlaySettingsFile::V3(file) => Some(file.settings.inner_mut()),
            _ => None,
        }
    }
}

/// Return raw effect entries for versioned settings files.
pub(crate) fn effects(play_settings: &PlaySettingsFile) -> Option<&[EffectSettings]> {
    play_settings
        .versioned_payload()
        .map(|payload| payload.effects.as_slice())
}

/// Return the first convolution-reverb effect payload object, if present.
pub(crate) fn first_convolution_reverb_settings(
    play_settings: &PlaySettingsFile,
) -> Option<&serde_json::Map<String, serde_json::Value>> {
    let effects = effects(play_settings)?;
    effects.iter().find_map(|effect| {
        let wrapper = effect.as_object()?;
        wrapper
            .get("ConvolutionReverbSettings")
            .and_then(serde_json::Value::as_object)
    })
}

/// Extract raw impulse-response setting text from play settings.
pub(crate) fn extract_impulse_response_text(play_settings: &PlaySettingsFile) -> Option<String> {
    let settings = first_convolution_reverb_settings(play_settings)?;
    settings
        .get("impulse_response")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            settings
                .get("impulse_response_attachment")
                .and_then(serde_json::Value::as_str)
        })
        .or_else(|| {
            settings
                .get("impulse_response_path")
                .and_then(serde_json::Value::as_str)
        })
        .map(ToString::to_string)
}

/// Extract configured convolution-reverb tail dB from play settings.
pub(crate) fn extract_impulse_response_tail_db(play_settings: &PlaySettingsFile) -> Option<f32> {
    let settings = first_convolution_reverb_settings(play_settings)?;
    settings
        .get("impulse_response_tail_db")
        .and_then(serde_json::Value::as_f64)
        .map(|value| value as f32)
        .or_else(|| {
            settings
                .get("impulse_response_tail")
                .and_then(serde_json::Value::as_f64)
                .map(|value| value as f32)
        })
}

impl<'de> Deserialize<'de> for PlaySettingsFile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let encoder_version = value.get("encoder_version").and_then(|raw| match raw {
            serde_json::Value::String(version) => Some(version.clone()),
            serde_json::Value::Number(number) => number
                .as_f64()
                .map(|val| {
                    if (val - 1.0).abs() < f64::EPSILON {
                        "1".to_string()
                    } else if (val - 2.0).abs() < f64::EPSILON {
                        "2".to_string()
                    } else if (val - 3.0).abs() < f64::EPSILON {
                        "3".to_string()
                    } else {
                        number.to_string()
                    }
                })
                .or_else(|| Some(number.to_string())),
            _ => None,
        });

        info!("encoder version: {:?}", encoder_version);

        let parsed = match encoder_version.as_deref() {
            None => serde_json::from_value::<PlaySettingsLegacyFile>(value.clone())
                .map(PlaySettingsFile::Legacy),
            Some("1") => serde_json::from_value::<PlaySettingsV1File>(value.clone())
                .map(PlaySettingsFile::V1),
            Some("2") => serde_json::from_value::<PlaySettingsV2File>(value.clone())
                .map(PlaySettingsFile::V2),
            Some("3") => serde_json::from_value::<PlaySettingsV3File>(value.clone())
                .map(PlaySettingsFile::V3),
            Some(version) => {
                warn!("unknown encoder version: {:?}", version);
                return Ok(PlaySettingsFile::Unknown { raw: value });
            }
        };

        parsed.or_else(|_| Ok(PlaySettingsFile::Unknown { raw: value }))
    }
}

impl Serialize for PlaySettingsFile {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        fn with_version<T, S>(payload: &T, version: &str, serializer: S) -> Result<S::Ok, S::Error>
        where
            T: Serialize,
            S: Serializer,
        {
            let mut value = serde_json::to_value(payload).map_err(serde::ser::Error::custom)?;
            match value {
                serde_json::Value::Object(ref mut map) => {
                    map.insert(
                        "encoder_version".to_string(),
                        serde_json::Value::String(version.to_string()),
                    );
                }
                other => {
                    let mut map = serde_json::Map::new();
                    map.insert(
                        "encoder_version".to_string(),
                        serde_json::Value::String(version.to_string()),
                    );
                    map.insert("play_settings".to_string(), other);
                    value = serde_json::Value::Object(map);
                }
            }
            value.serialize(serializer)
        }

        match self {
            PlaySettingsFile::Legacy(file) => file.serialize(serializer),
            PlaySettingsFile::V1(file) => with_version(file, "1", serializer),
            PlaySettingsFile::V2(file) => with_version(file, "2", serializer),
            PlaySettingsFile::V3(file) => with_version(file, "3", serializer),
            PlaySettingsFile::Unknown { raw, .. } => raw.serialize(serializer),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl PlaySettingsFile {
        fn encoder_version(&self) -> Option<&str> {
            match self {
                PlaySettingsFile::Legacy(_) => None,
                PlaySettingsFile::V1(_) => Some("1"),
                PlaySettingsFile::V2(_) => Some("2"),
                PlaySettingsFile::V3(_) => Some("3"),
                PlaySettingsFile::Unknown { raw } => {
                    raw.get("encoder_version").and_then(|v| v.as_str())
                }
            }
        }
    }

    #[test]
    fn container_inner_accessors_work_for_both_variants() {
        let mut nested = PlaySettingsContainer::Nested {
            play_settings: 1_u32,
        };
        assert_eq!(*nested.inner(), 1);
        *nested.inner_mut() = 2;
        assert_eq!(*nested.inner(), 2);

        let mut flat = PlaySettingsContainer::Flat(3_u32);
        assert_eq!(*flat.inner(), 3);
        *flat.inner_mut() = 4;
        assert_eq!(*flat.inner(), 4);
    }

    #[test]
    fn deserialize_versioned_settings_and_preserve_encoder_version_on_serialize() {
        let parsed: PlaySettingsFile = serde_json::from_str(
            r#"{
                "encoder_version": "1",
                "play_settings": { "effects": [], "tracks": [] }
            }"#,
        )
        .unwrap();

        assert_eq!(parsed.encoder_version(), Some("1"));
        let serialized = serde_json::to_value(parsed).unwrap();
        assert_eq!(serialized["encoder_version"], "1");
    }

    #[test]
    fn deserialize_unknown_encoder_version_as_unknown_variant() {
        let parsed: PlaySettingsFile =
            serde_json::from_str(r#"{"encoder_version": "99", "play_settings": {}}"#).unwrap();
        assert!(matches!(parsed, PlaySettingsFile::Unknown { .. }));
        assert_eq!(parsed.encoder_version(), Some("99"));
    }

    #[test]
    fn versioned_payload_defaults_to_empty_lists() {
        let v1: PlaySettingsV1 = serde_json::from_str("{}").unwrap();
        let v2: PlaySettingsV2 = serde_json::from_str("{}").unwrap();
        let v3: PlaySettingsV3 = serde_json::from_str("{}").unwrap();

        assert!(v1.effects.is_empty() && v1.tracks.is_empty());
        assert!(v2.effects.is_empty() && v2.tracks.is_empty());
        assert!(v3.effects.is_empty() && v3.tracks.is_empty());
    }
}
