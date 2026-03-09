//! Symphonia helpers for opening and decoding audio files.

use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Errors produced while opening or preparing decoder state for media input.
#[derive(Debug)]
pub enum DecoderOpenError {
    Io(std::io::Error),
    UnsupportedFormat(SymphoniaError),
    NoSupportedAudioTrack,
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

/// Open a file and return a decoder plus format reader.
///
/// This is a convenience wrapper around [`get_reader`] and [`get_decoder`].
pub fn open_file(
    file_path: &str,
) -> Result<(Box<dyn Decoder>, Box<dyn FormatReader>), DecoderOpenError> {
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

    // Find the first audio track with a known (decodeable) codec.
    if format
        .tracks()
        .iter()
        .all(|track| track.codec_params.codec == CODEC_TYPE_NULL)
    {
        return Err(DecoderOpenError::NoSupportedAudioTrack);
    }

    Ok(format)
}

/// Build a decoder for the first supported audio track in a `FormatReader`.
///
/// Uses the same track-selection logic as [`get_reader`]: finds the first track
/// with a non-null codec rather than blindly using `tracks()[0]`.
pub fn get_decoder(format: &dyn FormatReader) -> Result<Box<dyn Decoder>, DecoderOpenError> {
    let dec_opts: DecoderOptions = Default::default();

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or(DecoderOpenError::NoSupportedAudioTrack)?;

    symphonia::default::get_codecs()
        .make(&track.codec_params, &dec_opts)
        .map_err(DecoderOpenError::UnsupportedCodec)
}
