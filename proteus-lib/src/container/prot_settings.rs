//! Play-settings loading and effect-derivation helpers for `Prot`.

use log::{error, info, warn};
use matroska::Matroska;

use crate::container::play_settings::{self, PlaySettingsFile};
use crate::dsp::effects::convolution_reverb::{parse_impulse_response_string, ImpulseResponseSpec};
use crate::dsp::effects::{normalize_legacy_effect_aliases, AudioEffect};

/// Runtime settings extracted from parsed play-settings payloads.
#[derive(Debug, Clone, Default)]
pub struct ProtRuntimeSettings {
    pub impulse_response_spec: Option<ImpulseResponseSpec>,
    pub impulse_response_tail_db: Option<f32>,
    pub effects: Option<Vec<AudioEffect>>,
}

/// Load and parse `play_settings.json` from a `.prot`/`.mka` attachment list.
pub fn load_play_settings_from_container(file_path: &str) -> Option<PlaySettingsFile> {
    let file = std::fs::File::open(file_path).ok()?;
    let mka: Matroska = Matroska::open(file).ok()?;

    for attachment in &mka.attachments {
        if attachment.name != "play_settings.json" {
            continue;
        }

        match serde_json::from_slice::<PlaySettingsFile>(&attachment.data) {
            Ok(play_settings) => return Some(play_settings),
            Err(err) => {
                error!("Failed to parse play_settings.json: {}", err);
                return None;
            }
        }
    }

    None
}

/// Derive runtime effect state from a parsed play-settings file.
pub fn derive_runtime_settings(play_settings: &PlaySettingsFile) -> ProtRuntimeSettings {
    let impulse_response_spec = play_settings::extract_impulse_response_text(play_settings)
        .as_deref()
        .and_then(parse_impulse_response_string);
    let impulse_response_tail_db = play_settings::extract_impulse_response_tail_db(play_settings);

    let mut effects = None;
    if let Some(raw_effects) = play_settings::effects(play_settings) {
        let mut decoded = Vec::with_capacity(raw_effects.len());
        for effect in raw_effects {
            match serde_json::from_value::<AudioEffect>(effect.clone()) {
                Ok(effect) => decoded.push(effect),
                Err(err) => warn!("Failed to parse effect entry: {}", err),
            }
        }
        if !decoded.is_empty() {
            effects = Some(normalize_legacy_effect_aliases(decoded));
        }
    }

    info!("Parsed play_settings runtime settings");
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
