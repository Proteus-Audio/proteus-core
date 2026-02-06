use log::{info, warn};
use serde::{Deserialize, Deserializer};

pub mod legacy;
pub mod v1;
pub mod v2;

pub use legacy::{PlaySettingsLegacy, PlaySettingsLegacyFile, PlaySettingsTrackLegacy};
pub use v1::{PlaySettingsV1, PlaySettingsV1File};
pub use v2::{PlaySettingsV2, PlaySettingsV2File};

#[derive(Debug, Clone, Deserialize)]
pub struct ReverbSettings {
    pub decay: f32,
    pub pre_delay: f32,
    pub mix: f32,
    pub active: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompressorSettings {
    pub attack: f32,
    pub knee: f32,
    pub ratio: f32,
    pub release: f32,
    pub threshold: f32,
    pub active: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub enum EffectSettings {
    ReverbSettings(ReverbSettings),
    CompressorSettings(CompressorSettings),
    ConvolutionReverbSettings(ConvolutionReverbSettings),
}

#[derive(Debug, Clone, Deserialize)]
pub struct SettingsTrack {
    pub level: f32,
    pub pan: f32,
    pub ids: Vec<u32>,
    pub name: String,
    pub safe_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConvolutionReverbSettings {
    pub impulse_response: Option<String>,
    pub impulse_response_attachment: Option<String>,
    pub impulse_response_path: Option<String>,
    pub impulse_response_tail_db: Option<f32>,
    pub impulse_response_tail: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum PlaySettingsContainer<T> {
    Nested { play_settings: T },
    Flat(T),
}

impl<T> PlaySettingsContainer<T> {
    pub fn inner(&self) -> &T {
        match self {
            PlaySettingsContainer::Nested { play_settings } => play_settings,
            PlaySettingsContainer::Flat(inner) => inner,
        }
    }
}

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
