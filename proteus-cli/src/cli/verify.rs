//! CLI helpers for probe/decode verification without playback.

use std::io;

use log::{error, info, warn};
use symphonia::core::codecs::{Decoder, CODEC_TYPE_NULL};
use symphonia::core::errors::{Error, Result};
use symphonia::core::formats::FormatReader;

use proteus_lib::container::info::get_probe_result_from_string;
use proteus_lib::tools::decode::{check_audio_file_supported, get_decoder, get_reader};

/// Modes for non-playback verification.
#[derive(Debug, Clone, Copy)]
pub enum VerifyMode {
    Decode,
    Probe,
    Supported,
    Verify,
}

/// Run a verify subcommand mode for the given input file.
pub fn run_verify(file_path: &str, mode: VerifyMode) -> Result<i32> {
    match mode {
        VerifyMode::Probe => run_probe(file_path),
        VerifyMode::Supported => run_supported(file_path),
        VerifyMode::Decode => run_decode(file_path, false),
        VerifyMode::Verify => run_decode(file_path, true),
    }
}

fn run_supported(file_path: &str) -> Result<i32> {
    let check = check_audio_file_supported(file_path);
    if check.supported {
        info!(
            "Supported audio file (audio_tracks={})",
            check.audio_track_count
        );
        Ok(0)
    } else {
        error!(
            "Unsupported audio file{}",
            check
                .reason
                .as_deref()
                .map(|reason| format!(": {}", reason))
                .unwrap_or_default()
        );
        Ok(1)
    }
}

fn run_probe(file_path: &str) -> Result<i32> {
    let probed = get_probe_result_from_string(file_path)?;
    let tracks = probed.format.tracks();
    info!("Probed {} track(s)", tracks.len());
    for track in tracks {
        let params = &track.codec_params;
        let codec = params.codec;
        let sample_rate = params.sample_rate.unwrap_or(0);
        let channels = params.channels.map(|c| c.count()).unwrap_or(0);
        let bits = params.bits_per_sample.unwrap_or(0);
        info!(
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

type DecoderOpenResult = (Box<dyn Decoder>, Box<dyn FormatReader>, u32);

fn open_decoder(file_path: &str) -> Result<DecoderOpenResult> {
    let format = get_reader(file_path).map_err(|err| Error::IoError(io::Error::other(err)))?;

    let track_id = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .map(|track| track.id)
        .ok_or(Error::Unsupported("no supported audio tracks"))?;

    let decoder =
        get_decoder(format.as_ref()).map_err(|err| Error::IoError(io::Error::other(err)))?;
    Ok((decoder, format, track_id))
}

#[cfg(test)]
mod tests {
    use super::{run_verify, VerifyMode};

    #[test]
    fn invalid_path_returns_error_for_all_modes() {
        let missing = "/definitely/missing/audio.file";
        assert!(run_verify(missing, VerifyMode::Probe).is_err());
        assert!(run_verify(missing, VerifyMode::Decode).is_err());
        assert!(run_verify(missing, VerifyMode::Verify).is_err());
    }

    #[test]
    fn supported_mode_accepts_ogg_opus_fixture() {
        let file_path = format!(
            "{}/../test_audio/deep_trouble_000.ogg",
            env!("CARGO_MANIFEST_DIR")
        );
        let code = run_verify(&file_path, VerifyMode::Supported).expect("check should run");
        assert_eq!(code, 0);
    }
}
