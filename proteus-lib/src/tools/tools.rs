//! Symphonia helpers for opening and decoding audio files.

use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Open a file and return a decoder plus format reader.
///
/// This is a convenience wrapper around [`get_reader`] and [`get_decoder`].
pub fn open_file(file_path: &str) -> (Box<dyn Decoder>, Box<dyn FormatReader>) {
    let format = get_reader(file_path);
    let decoder = get_decoder(&format);

    (decoder, format)
}

/// Build a Symphonia `FormatReader` for the given file path.
///
/// `.prot` files are treated as `.mka` for probe hinting.
pub fn get_reader(file_path: &str) -> Box<dyn FormatReader> {
    // Open the media source.
    let src = std::fs::File::open(file_path).expect("failed to open media");

    // Create the media source stream.
    let mss = MediaSourceStream::new(Box::new(src), Default::default());

    // Create a probe hint using the file's extension. [Optional]
    let mut hint = Hint::new();
    let mut hint_extension = std::path::Path::new(file_path)
        .extension()
        .unwrap()
        .to_str()
        .unwrap();
    // if hint_extension == "prot" replace with "mka"
    if hint_extension == "prot" {
        hint_extension = "mka";
    }
    hint.with_extension(hint_extension);

    // Use the default options for metadata and format readers.
    let meta_opts: MetadataOptions = Default::default();
    let fmt_opts: FormatOptions = Default::default();

    // Probe the media source.
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &meta_opts)
        .expect("unsupported format");

    // Get the instantiated format reader.
    let format = probed.format;

    // Find the first audio track with a known (decodeable) codec.
    format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .expect("no supported audio tracks");

    format
}

/// Build a decoder for the first audio track in a `FormatReader`.
pub fn get_decoder(format: &Box<dyn FormatReader>) -> Box<dyn Decoder> {
    // Use the default options for the decoder.
    let dec_opts: DecoderOptions = Default::default();

    // Create a decoder for the track.
    let decoder = symphonia::default::get_codecs()
        .make(&format.tracks()[0].codec_params, &dec_opts)
        .expect("unsupported codec");

    decoder
}
