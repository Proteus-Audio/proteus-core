//! Serde models for `play_settings.json` with versioned decoding.

use log::{info, warn};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub mod legacy;
pub mod v1;
pub mod v2;
pub mod v3;

pub use legacy::{PlaySettingsLegacy, PlaySettingsLegacyFile, PlaySettingsTrackLegacy};
pub use v1::{PlaySettingsV1, PlaySettingsV1File};
pub use v2::{PlaySettingsV2, PlaySettingsV2File};
pub use v3::{PlaySettingsV3, PlaySettingsV3File};

/// Effect settings variants that can appear in the settings file.
pub type EffectSettings = serde_json::Value;

/// Track-level configuration shared by newer settings versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsTrack {
    pub level: f32,
    pub pan: f32,
    pub ids: Vec<u32>,
    pub name: String,
    pub safe_name: String,
    #[serde(default = "default_selections_count")]
    pub selections_count: u32,
    #[serde(default)]
    pub shuffle_points: Vec<String>,
}

/// Shared payload used by versioned `play_settings.json` schemas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaySettingsPayload {
    #[serde(default)]
    pub effects: Vec<EffectSettings>,
    #[serde(default)]
    pub tracks: Vec<SettingsTrack>,
}

/// Top-level wrapper shared by versioned settings files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionedPlaySettingsFile<T> {
    #[serde(flatten)]
    pub settings: PlaySettingsContainer<T>,
}

fn default_selections_count() -> u32 {
    1
}

/// Wrapper allowing `play_settings` to be nested or flat.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PlaySettingsContainer<T> {
    Nested { play_settings: T },
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
pub enum PlaySettingsFile {
    Legacy(PlaySettingsLegacyFile),
    V1(PlaySettingsV1File),
    V2(PlaySettingsV2File),
    V3(PlaySettingsV3File),
    Unknown {
        encoder_version: Option<String>,
        raw: serde_json::Value,
    },
}

impl PlaySettingsFile {
    /// Get the encoder version string, if known.
    pub fn encoder_version(&self) -> Option<&str> {
        match self {
            PlaySettingsFile::Legacy(_) => None,
            PlaySettingsFile::V1(_) => Some("1"),
            PlaySettingsFile::V2(_) => Some("2"),
            PlaySettingsFile::V3(_) => Some("3"),
            PlaySettingsFile::Unknown {
                encoder_version, ..
            } => encoder_version.as_deref(),
        }
    }

    /// Return normalized modern payload for V1/V2/V3 settings.
    pub fn versioned_payload(&self) -> Option<&PlaySettingsPayload> {
        match self {
            PlaySettingsFile::V1(file) => Some(file.settings.inner()),
            PlaySettingsFile::V2(file) => Some(file.settings.inner()),
            PlaySettingsFile::V3(file) => Some(file.settings.inner()),
            _ => None,
        }
    }

    /// Return mutable normalized modern payload for V1/V2/V3 settings.
    pub fn versioned_payload_mut(&mut self) -> Option<&mut PlaySettingsPayload> {
        match self {
            PlaySettingsFile::V1(file) => Some(file.settings.inner_mut()),
            PlaySettingsFile::V2(file) => Some(file.settings.inner_mut()),
            PlaySettingsFile::V3(file) => Some(file.settings.inner_mut()),
            _ => None,
        }
    }
}

/// Return raw effect entries for versioned settings files.
pub fn effects(play_settings: &PlaySettingsFile) -> Option<&[EffectSettings]> {
    play_settings
        .versioned_payload()
        .map(|payload| payload.effects.as_slice())
}

/// Return the first convolution-reverb effect payload object, if present.
pub fn first_convolution_reverb_settings(
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
pub fn extract_impulse_response_text(play_settings: &PlaySettingsFile) -> Option<String> {
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
pub fn extract_impulse_response_tail_db(play_settings: &PlaySettingsFile) -> Option<f32> {
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

        info!("Encoder version: {:?}", encoder_version);

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
                warn!("Unknown encoder version: {:?}", version);
                return Ok(PlaySettingsFile::Unknown {
                    encoder_version,
                    raw: value,
                });
            }
        };

        parsed.or_else(|_| {
            Ok(PlaySettingsFile::Unknown {
                encoder_version,
                raw: value,
            })
        })
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
