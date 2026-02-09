//! Impulse response specification and play-settings parsing helpers.

use crate::container::play_settings::{
    ConvolutionReverbSettings, EffectSettings, PlaySettingsFile,
};

/// Location of an impulse response used for convolution reverb.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImpulseResponseSpec {
    Attachment(String),
    FilePath(String),
}

/// Parse a convolution reverb impulse response spec from play settings.
pub(crate) fn parse_impulse_response_spec(
    play_settings: &PlaySettingsFile,
) -> Option<ImpulseResponseSpec> {
    if let Some(settings) = parse_convolution_settings(play_settings) {
        if let Some(spec) = parse_impulse_response_string_or_struct(&settings) {
            return Some(spec);
        }
    }

    None
}

/// Parse the convolution reverb tail trim (dB) from play settings.
pub(crate) fn parse_impulse_response_tail_db(play_settings: &PlaySettingsFile) -> Option<f32> {
    if let Some(settings) = parse_convolution_settings(play_settings) {
        if let Some(value) = settings.impulse_response_tail_db {
            return Some(value);
        }
        if let Some(value) = settings.impulse_response_tail {
            return Some(value);
        }
    }

    None
}

fn parse_convolution_settings(
    play_settings: &PlaySettingsFile,
) -> Option<ConvolutionReverbSettings> {
    let effects = match play_settings {
        PlaySettingsFile::V1(file) => &file.settings.inner().effects,
        PlaySettingsFile::V2(file) => &file.settings.inner().effects,
        _ => return None,
    };

    for effect in effects {
        if let EffectSettings::ConvolutionReverb(effect) = effect {
            return Some(effect.settings.clone());
        }
    }

    None
}

pub(crate) fn parse_impulse_response_string_or_struct(
    settings: &ConvolutionReverbSettings,
) -> Option<ImpulseResponseSpec> {
    if let Some(value) = settings.impulse_response.as_deref() {
        return parse_impulse_response_string(value);
    }
    if let Some(value) = settings.impulse_response_attachment.as_deref() {
        return parse_impulse_response_string(value);
    }
    if let Some(value) = settings.impulse_response_path.as_deref() {
        return parse_impulse_response_string(value);
    }
    None
}

/// Parse an impulse response spec string into a concrete location.
///
/// Supported prefixes:
/// - `attachment:` for container attachments
/// - `file:` for explicit file paths
pub fn parse_impulse_response_string(value: &str) -> Option<ImpulseResponseSpec> {
    if let Some(attachment) = value.strip_prefix("attachment:") {
        return Some(ImpulseResponseSpec::Attachment(
            attachment.trim().to_string(),
        ));
    }

    if let Some(path) = value.strip_prefix("file:") {
        return Some(ImpulseResponseSpec::FilePath(path.trim().to_string()));
    }

    Some(ImpulseResponseSpec::FilePath(value.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::play_settings::{
        PlaySettingsContainer, PlaySettingsFile, PlaySettingsV2, PlaySettingsV2File,
    };
    use crate::dsp::effects::{AudioEffect, ConvolutionReverbEffect};

    #[test]
    fn parse_impulse_response_string_variants() {
        assert_eq!(
            parse_impulse_response_string("attachment:foo.wav"),
            Some(ImpulseResponseSpec::Attachment("foo.wav".to_string()))
        );
        assert_eq!(
            parse_impulse_response_string("file:/tmp/bar.wav"),
            Some(ImpulseResponseSpec::FilePath("/tmp/bar.wav".to_string()))
        );
        assert_eq!(
            parse_impulse_response_string("plain.wav"),
            Some(ImpulseResponseSpec::FilePath("plain.wav".to_string()))
        );
    }

    #[test]
    fn parse_impulse_response_from_play_settings() {
        let mut effect = ConvolutionReverbEffect::default();
        effect.settings.impulse_response = Some("attachment:ir.wav".to_string());
        effect.settings.impulse_response_tail_db = Some(-42.0);

        let settings = PlaySettingsV2 {
            tracks: Vec::new(),
            effects: vec![AudioEffect::ConvolutionReverb(effect)],
        };
        let file = PlaySettingsV2File {
            settings: PlaySettingsContainer::Flat(settings),
        };

        let play_settings = PlaySettingsFile::V2(file);
        let spec = parse_impulse_response_spec(&play_settings);
        assert_eq!(
            spec,
            Some(ImpulseResponseSpec::Attachment("ir.wav".to_string()))
        );
        let tail_db = parse_impulse_response_tail_db(&play_settings);
        assert_eq!(tail_db, Some(-42.0));
    }
}
