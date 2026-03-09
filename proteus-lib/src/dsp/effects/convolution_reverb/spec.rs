//! Impulse response specification parsing helpers.

/// Location of an impulse response used for convolution reverb.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImpulseResponseSpec {
    Attachment(String),
    FilePath(String),
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
}
