//! Play-settings loading and effect-derivation helpers for `Prot`.

use log::{info, warn};
use matroska::Matroska;

use crate::container::play_settings::{self, PlaySettingsFile};
use crate::dsp::effects::convolution_reverb::{parse_impulse_response_string, ImpulseResponseSpec};
use crate::dsp::effects::{normalize_legacy_effect_aliases, AudioEffect};

/// Runtime settings extracted from parsed play-settings payloads.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProtRuntimeSettings {
    /// Resolved specification for the impulse response used by convolution reverb.
    pub impulse_response_spec: Option<ImpulseResponseSpec>,
    /// dB level at which the IR tail is considered silent and can be truncated.
    pub impulse_response_tail_db: Option<f32>,
    /// DSP effect chain extracted from the settings file, if present.
    pub effects: Option<Vec<AudioEffect>>,
}

/// Failure modes while loading `play_settings.json` from a container file.
#[derive(Debug)]
pub(crate) enum PlaySettingsLoadError {
    /// Failed to open the container file for reading.
    OpenFile(std::io::Error),
    /// Failed to parse the file as a Matroska container.
    OpenMatroska(matroska::Error),
    /// Failed to deserialize the `play_settings.json` attachment as JSON.
    ParseJson(serde_json::Error),
    /// The `play_settings.json` attachment was not present in the container.
    MissingAttachment,
}

impl std::fmt::Display for PlaySettingsLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenFile(err) => write!(f, "failed to open container file: {}", err),
            Self::OpenMatroska(err) => write!(f, "failed to read matroska container: {}", err),
            Self::ParseJson(err) => write!(f, "failed to parse play_settings.json: {}", err),
            Self::MissingAttachment => write!(f, "play_settings.json attachment not found"),
        }
    }
}

impl std::error::Error for PlaySettingsLoadError {}

/// Fallible play-settings loader with typed error variants.
pub(crate) fn try_load_play_settings_from_container(
    file_path: &str,
) -> Result<PlaySettingsFile, PlaySettingsLoadError> {
    let file = std::fs::File::open(file_path).map_err(PlaySettingsLoadError::OpenFile)?;
    let mka: Matroska = Matroska::open(file).map_err(PlaySettingsLoadError::OpenMatroska)?;

    let attachment = mka
        .attachments
        .iter()
        .find(|attachment| attachment.name == "play_settings.json")
        .ok_or(PlaySettingsLoadError::MissingAttachment)?;

    serde_json::from_slice::<PlaySettingsFile>(&attachment.data)
        .map_err(PlaySettingsLoadError::ParseJson)
}

/// Derive runtime effect state from a parsed play-settings file.
pub(crate) fn derive_runtime_settings(play_settings: &PlaySettingsFile) -> ProtRuntimeSettings {
    let impulse_response_spec = play_settings::extract_impulse_response_text(play_settings)
        .as_deref()
        .and_then(parse_impulse_response_string);
    let impulse_response_tail_db = play_settings::extract_impulse_response_tail_db(play_settings);

    let mut effects = None;
    if let Some(raw_effects) = play_settings::effects(play_settings) {
        let mut decoded = Vec::with_capacity(raw_effects.len());
        for effect in raw_effects {
            match effect.decode_audio_effect() {
                Ok(effect) => decoded.push(effect),
                Err(err) => warn!("failed to parse effect entry: {}", err),
            }
        }
        if !decoded.is_empty() {
            effects = Some(normalize_legacy_effect_aliases(decoded));
        }
    }

    info!("parsed play_settings runtime settings");
    ProtRuntimeSettings {
        impulse_response_spec,
        impulse_response_tail_db,
        effects,
    }
}

#[cfg(test)]
mod tests {
    use super::derive_runtime_settings;
    use crate::container::play_settings::PlaySettingsFile;
    use crate::dsp::effects::convolution_reverb::ImpulseResponseSpec;

    #[test]
    fn derive_runtime_settings_reads_impulse_response_fields() {
        let play_settings: PlaySettingsFile = serde_json::from_str(
            r#"{
                "encoder_version":"2",
                "play_settings":{
                    "effects":[
                        {
                            "ConvolutionReverbSettings":{
                                "enabled":true,
                                "impulse_response":"attachment:hall.wav",
                                "impulse_response_tail_db":-24.0
                            }
                        }
                    ],
                    "tracks":[]
                }
            }"#,
        )
        .unwrap();

        let runtime = derive_runtime_settings(&play_settings);
        assert_eq!(
            runtime.impulse_response_spec,
            Some(ImpulseResponseSpec::Attachment("hall.wav".to_string()))
        );
        assert_eq!(runtime.impulse_response_tail_db, Some(-24.0));
        assert!(runtime.effects.is_some());
    }

    #[test]
    fn derive_runtime_settings_handles_missing_effects() {
        let play_settings: PlaySettingsFile = serde_json::from_str(
            r#"{"encoder_version":"3","play_settings":{"effects":[],"tracks":[]}}"#,
        )
        .unwrap();
        let runtime = derive_runtime_settings(&play_settings);
        assert_eq!(runtime.impulse_response_spec, None);
        assert_eq!(runtime.impulse_response_tail_db, None);
        assert!(runtime.effects.is_none());
    }
}
