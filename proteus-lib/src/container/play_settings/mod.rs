//! Serde models for `play_settings.json` with versioned decoding.

use log::{info, warn};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub mod legacy;
pub mod v1;
pub mod v2;

pub use legacy::{PlaySettingsLegacy, PlaySettingsLegacyFile, PlaySettingsTrackLegacy};
pub use v1::{PlaySettingsV1, PlaySettingsV1File};
pub use v2::{PlaySettingsV2, PlaySettingsV2File};

/// Legacy algorithmic reverb configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReverbSettings {
    pub decay: f32,
    pub pre_delay: f32,
    pub mix: f32,
    pub active: bool,
}

/// Legacy compressor configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressorSettings {
    pub attack: f32,
    pub knee: f32,
    pub ratio: f32,
    pub release: f32,
    pub threshold: f32,
    pub active: bool,
}

/// Effect settings variants that can appear in the settings file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EffectSettings {
    ReverbSettings(ReverbSettings),
    CompressorSettings(CompressorSettings),
    ConvolutionReverbSettings(ConvolutionReverbSettings),
}

/// Track-level configuration shared by newer settings versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsTrack {
    pub level: f32,
    pub pan: f32,
    pub ids: Vec<u32>,
    pub name: String,
    pub safe_name: String,
}

/// Convolution reverb configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvolutionReverbSettings {
    pub impulse_response: Option<String>,
    pub impulse_response_attachment: Option<String>,
    pub impulse_response_path: Option<String>,
    pub impulse_response_tail_db: Option<f32>,
    pub impulse_response_tail: Option<f32>,
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
}

/// Versioned settings file representation.
#[derive(Debug, Clone)]
pub enum PlaySettingsFile {
    Legacy(PlaySettingsLegacyFile),
    V1(PlaySettingsV1File),
    V2(PlaySettingsV2File),
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
            PlaySettingsFile::Unknown {
                encoder_version, ..
            } => encoder_version.as_deref(),
        }
    }
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
            let mut value =
                serde_json::to_value(payload).map_err(serde::ser::Error::custom)?;
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
            PlaySettingsFile::Unknown { raw, .. } => raw.serialize(serializer),
        }
    }
}
