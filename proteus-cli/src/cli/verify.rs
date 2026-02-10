//! CLI helpers for probe/decode verification without playback.

use std::fs::File;
use std::io;

use log::{error, info, warn};
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::{Error, Result};
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use proteus_lib::container::info::get_probe_result_from_string;

/// Modes for non-playback verification.
#[derive(Debug, Clone, Copy)]
pub enum VerifyMode {
    DecodeOnly,
    ProbeOnly,
    VerifyOnly,
}

/// Run a verify subcommand mode for the given input file.
pub fn run_verify(file_path: &str, mode: VerifyMode) -> Result<i32> {
    match mode {
        VerifyMode::ProbeOnly => run_probe(file_path),
        VerifyMode::DecodeOnly => run_decode(file_path, false),
        VerifyMode::VerifyOnly => run_decode(file_path, true),
    }
}

fn run_probe(file_path: &str) -> Result<i32> {
    let probed = get_probe_result_from_string(file_path)?;
    let tracks = probed.format.tracks();
    println!("Probed {} track(s)", tracks.len());
    for track in tracks {
        let params = &track.codec_params;
        let codec = params.codec;
        let sample_rate = params.sample_rate.unwrap_or(0);
        let channels = params.channels.map(|c| c.count()).unwrap_or(0);
        let bits = params.bits_per_sample.unwrap_or(0);
        println!(
            "track {} codec={:?} sample_rate={} channels={} bits_per_sample={}",
            track.id, codec, sample_rate, channels, bits
        );
    }
    Ok(0)
}

fn run_decode(file_path: &str, strict: bool) -> Result<i32> {
    let (mut decoder, mut format, track_id) = open_decoder(file_path)?;
    let mut packets = 0_u64;
    let mut decode_errors = 0_u64;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(err)) if err.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(err),
        };

        if packet.track_id() != track_id {
            continue;
        }

        packets = packets.saturating_add(1);
        match decoder.decode(&packet) {
            Ok(_) => {}
            Err(Error::DecodeError(err)) => {
                decode_errors = decode_errors.saturating_add(1);
                warn!("decode error: {}", err);
            }
            Err(Error::IoError(err)) if err.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(err),
        }
    }

    info!(
        "Decoded {} packet(s) with {} decode error(s)",
        packets, decode_errors
    );

    if packets == 0 {
        error!("No packets decoded");
        return Ok(1);
    }

    if strict && decode_errors > 0 {
        error!("Decode verification failed with {} error(s)", decode_errors);
        return Ok(1);
    }

    Ok(0)
}

fn open_decoder(file_path: &str) -> Result<(Box<dyn Decoder>, Box<dyn FormatReader>, u32)> {
    let src = File::open(file_path).map_err(Error::IoError)?;
    let mss = MediaSourceStream::new(Box::new(src), Default::default());

    let mut hint = Hint::new();
    if let Some(extension) = std::path::Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
    {
        let hint_extension = if extension == "prot" { "mka" } else { extension };
        hint.with_extension(hint_extension);
    }

    let meta_opts: MetadataOptions = Default::default();
    let fmt_opts: FormatOptions = Default::default();
    let probed = symphonia::default::get_probe().format(&hint, mss, &fmt_opts, &meta_opts)?;
    let format = probed.format;

    let (track_id, codec_params) = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .map(|track| (track.id, track.codec_params.clone()))
        .ok_or_else(|| Error::Unsupported("no supported audio tracks"))?;

    let dec_opts: DecoderOptions = Default::default();
    let decoder = symphonia::default::get_codecs().make(&codec_params, &dec_opts)?;
    Ok((decoder, format, track_id))
}
