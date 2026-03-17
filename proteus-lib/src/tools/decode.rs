//! Symphonia helpers for opening and decoding audio files.

use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, Track};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Errors produced while opening or preparing decoder state for media input.
#[derive(Debug)]
pub enum DecoderOpenError {
    /// An I/O error occurred while opening the media source.
    Io(std::io::Error),
    /// Symphonia could not recognize the container format.
    UnsupportedFormat(SymphoniaError),
    /// The media source contained no audio track with a supported codec.
    NoSupportedAudioTrack,
    /// Symphonia could not construct a decoder for the audio codec.
    UnsupportedCodec(SymphoniaError),
}

impl std::fmt::Display for DecoderOpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "failed to open media file: {}", err),
            Self::UnsupportedFormat(err) => write!(f, "unsupported media format: {}", err),
            Self::NoSupportedAudioTrack => write!(f, "no supported audio tracks found"),
            Self::UnsupportedCodec(err) => write!(f, "unsupported audio codec: {}", err),
        }
    }
}

impl std::error::Error for DecoderOpenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::UnsupportedFormat(err) => Some(err),
            Self::NoSupportedAudioTrack => None,
            Self::UnsupportedCodec(err) => Some(err),
        }
    }
}

impl From<std::io::Error> for DecoderOpenError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

/// Shared opened decoder state for one media file.
pub type OpenedDecoder = (Box<dyn Decoder>, Box<dyn FormatReader>);

/// Open a file and return a decoder plus format reader.
///
/// This is a convenience wrapper around [`get_reader`] and [`get_decoder`].
pub fn open_file(file_path: &str) -> Result<OpenedDecoder, DecoderOpenError> {
    let format = get_reader(file_path)?;
    let decoder = get_decoder(format.as_ref())?;

    Ok((decoder, format))
}

/// Build a Symphonia `FormatReader` for the given file path.
///
/// `.prot` files are treated as `.mka` for probe hinting.
pub fn get_reader(file_path: &str) -> Result<Box<dyn FormatReader>, DecoderOpenError> {
    // Open the media source.
    let src = std::fs::File::open(file_path)?;

    // Create the media source stream.
    let mss = MediaSourceStream::new(Box::new(src), Default::default());

    // Create a probe hint using the file's extension. [Optional]
    let mut hint = Hint::new();
    if let Some(mut hint_extension) = std::path::Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
    {
        // if hint_extension == "prot" replace with "mka"
        if hint_extension == "prot" {
            hint_extension = "mka";
        }
        hint.with_extension(hint_extension);
    }

    // Use the default options for metadata and format readers.
    let meta_opts: MetadataOptions = Default::default();
    let fmt_opts: FormatOptions = Default::default();

    // Probe the media source.
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &meta_opts)
        .map_err(DecoderOpenError::UnsupportedFormat)?;

    // Get the instantiated format reader.
    let format = probed.format;

    // Verify at least one track has a decodable codec.
    find_audio_track(format.tracks())?;

    Ok(format)
}

/// Find the first track with a non-null (decodable) codec.
///
/// Returns the track reference, or [`DecoderOpenError::NoSupportedAudioTrack`]
/// if every track has `CODEC_TYPE_NULL`.
fn find_audio_track(tracks: &[Track]) -> Result<&Track, DecoderOpenError> {
    tracks
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or(DecoderOpenError::NoSupportedAudioTrack)
}

/// Build a decoder for the first supported audio track in a `FormatReader`.
///
/// Uses the same track-selection logic as [`get_reader`]: finds the first track
/// with a non-null codec rather than blindly using `tracks()[0]`.
pub fn get_decoder(format: &dyn FormatReader) -> Result<Box<dyn Decoder>, DecoderOpenError> {
    let dec_opts: DecoderOptions = Default::default();

    let track = find_audio_track(format.tracks())?;

    symphonia::default::get_codecs()
        .make(&track.codec_params, &dec_opts)
        .map_err(DecoderOpenError::UnsupportedCodec)
}

#[cfg(test)]
mod tests {
    use symphonia::core::codecs::{CodecParameters, CODEC_TYPE_NULL, CODEC_TYPE_VORBIS};
    use symphonia::core::formats::Track;

    use super::{find_audio_track, get_reader, DecoderOpenError};

    fn null_track(id: u32) -> Track {
        let mut params = CodecParameters::default();
        params.codec = CODEC_TYPE_NULL;
        Track::new(id, params)
    }

    fn audio_track(id: u32) -> Track {
        let mut params = CodecParameters::default();
        params.codec = CODEC_TYPE_VORBIS;
        Track::new(id, params)
    }

    #[test]
    fn find_audio_track_skips_null_first_track() {
        let tracks = [null_track(0), audio_track(1)];
        let track = find_audio_track(&tracks).expect("should find decodable track");
        assert_eq!(track.id, 1, "must select the later decodable track, not tracks()[0]");
    }

    #[test]
    fn find_audio_track_skips_multiple_null_tracks() {
        let tracks = [null_track(0), null_track(1), null_track(2), audio_track(3)];
        let track = find_audio_track(&tracks).expect("should find decodable track");
        assert_eq!(track.id, 3);
    }

    #[test]
    fn find_audio_track_returns_first_when_already_decodable() {
        let tracks = [audio_track(0), audio_track(1)];
        let track = find_audio_track(&tracks).expect("should find decodable track");
        assert_eq!(track.id, 0);
    }

    #[test]
    fn find_audio_track_all_null_returns_no_supported_error() {
        let tracks = [null_track(0), null_track(1)];
        let err = find_audio_track(&tracks).unwrap_err();
        assert!(matches!(err, DecoderOpenError::NoSupportedAudioTrack));
    }

    #[test]
    fn find_audio_track_empty_returns_no_supported_error() {
        let err = find_audio_track(&[]).unwrap_err();
        assert!(matches!(err, DecoderOpenError::NoSupportedAudioTrack));
    }

    #[test]
    fn get_reader_returns_io_error_for_missing_file() {
        let err = match get_reader("/definitely/missing/file.mp3") {
            Ok(_) => panic!("missing file should error"),
            Err(err) => err,
        };
        assert!(matches!(err, DecoderOpenError::Io(_)));
    }

    #[test]
    fn decoder_open_error_display_mentions_open_failure() {
        let err = match get_reader("/definitely/missing/file.wav") {
            Ok(_) => panic!("missing file should error"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("failed to open media file"));
    }
}
