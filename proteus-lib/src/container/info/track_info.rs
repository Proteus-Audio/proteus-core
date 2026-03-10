//! Track metadata extraction and compatibility checking.

use log::{debug, warn};

use symphonia::core::sample::SampleFormat;
use symphonia::core::{
    audio::{AudioBufferRef, Channels, Layout},
    codecs::{DecoderOptions, CODEC_TYPE_NULL},
    errors::Error,
    formats::Track,
};

use super::aiff::fallback_track_info;
use super::{get_probe_result_from_string, InfoError, TrackInfo};

pub(super) fn get_track_info(track: &Track) -> TrackInfo {
    let codec_params = &track.codec_params;
    let sample_rate = codec_params.sample_rate.unwrap_or(0);
    let bits_per_sample = codec_params
        .bits_per_sample
        .unwrap_or_else(|| bits_from_sample_format(codec_params.sample_format));

    let mut channel_count = match codec_params.channel_layout {
        Some(Layout::Mono) => 1,
        Some(Layout::Stereo) => 2,
        Some(Layout::TwoPointOne) => 3,
        Some(Layout::FivePointOne) => 6,
        _ => 0,
    };

    if channel_count == 0 {
        channel_count = codec_params
            .channels
            .unwrap_or(Channels::FRONT_CENTRE)
            .iter()
            .count() as u32;
    }

    TrackInfo {
        sample_rate,
        channel_count,
        bits_per_sample,
    }
}

fn bits_from_sample_format(sample_format: Option<SampleFormat>) -> u32 {
    match sample_format {
        Some(SampleFormat::U8 | SampleFormat::S8) => 8,
        Some(SampleFormat::U16 | SampleFormat::S16) => 16,
        Some(SampleFormat::U24 | SampleFormat::S24) => 24,
        Some(SampleFormat::U32 | SampleFormat::S32 | SampleFormat::F32) => 32,
        Some(SampleFormat::F64) => 64,
        None => 0,
    }
}

fn bits_from_decode(file_path: &str) -> u32 {
    let mut probed = match get_probe_result_from_string(file_path) {
        Ok(probed) => probed,
        Err(_) => return 0,
    };

    let (track_id, codec_params) = match probed
        .format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
    {
        Some(track) => (track.id, track.codec_params.clone()),
        None => return 0,
    };

    let dec_opts: DecoderOptions = Default::default();
    let mut decoder = match symphonia::default::get_codecs().make(&codec_params, &dec_opts) {
        Ok(decoder) => decoder,
        Err(_) => return 0,
    };

    loop {
        let packet = match probed.format.next_packet() {
            Ok(packet) => packet,
            Err(_) => return 0,
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                return match decoded {
                    AudioBufferRef::U8(_) => 8,
                    AudioBufferRef::S8(_) => 8,
                    AudioBufferRef::U16(_) => 16,
                    AudioBufferRef::S16(_) => 16,
                    AudioBufferRef::U24(_) => 24,
                    AudioBufferRef::S24(_) => 24,
                    AudioBufferRef::U32(_) => 32,
                    AudioBufferRef::S32(_) => 32,
                    AudioBufferRef::F32(_) => 32,
                    AudioBufferRef::F64(_) => 64,
                };
            }
            Err(Error::DecodeError(_)) => continue,
            Err(_) => return 0,
        }
    }
}

pub(super) fn reduce_track_infos(track_infos: Vec<TrackInfo>) -> Result<TrackInfo, InfoError> {
    if track_infos.is_empty() {
        return Ok(TrackInfo {
            sample_rate: 0,
            channel_count: 0,
            bits_per_sample: 0,
        });
    }

    let info =
        track_infos
            .into_iter()
            .try_fold(None::<TrackInfo>, |acc, track_info| match acc {
                Some(acc) => {
                    if acc.sample_rate != 0
                        && track_info.sample_rate != 0
                        && acc.sample_rate != track_info.sample_rate
                    {
                        return Err(InfoError::IncompatibleTracks(format!(
                            "sample rates do not match: {} != {}",
                            acc.sample_rate, track_info.sample_rate
                        )));
                    }

                    if acc.channel_count != 0
                        && track_info.channel_count != 0
                        && acc.channel_count != track_info.channel_count
                    {
                        return Err(InfoError::IncompatibleTracks(format!(
                            "channel counts do not match: {} != {}",
                            acc.channel_count, track_info.channel_count
                        )));
                    }

                    if acc.bits_per_sample != 0
                        && track_info.bits_per_sample != 0
                        && acc.bits_per_sample != track_info.bits_per_sample
                    {
                        return Err(InfoError::IncompatibleTracks(format!(
                            "bits per sample do not match: {} != {}",
                            acc.bits_per_sample, track_info.bits_per_sample
                        )));
                    }

                    Ok(Some(TrackInfo {
                        sample_rate: if acc.sample_rate == 0 {
                            track_info.sample_rate
                        } else {
                            acc.sample_rate
                        },
                        channel_count: if acc.channel_count == 0 {
                            track_info.channel_count
                        } else {
                            acc.channel_count
                        },
                        bits_per_sample: if acc.bits_per_sample == 0 {
                            track_info.bits_per_sample
                        } else {
                            acc.bits_per_sample
                        },
                    }))
                }
                None => Ok(Some(track_info)),
            })?;

    // Safe: is_empty() was checked above, so the fold processed at least one item.
    Ok(info.unwrap_or(TrackInfo {
        sample_rate: 0,
        channel_count: 0,
        bits_per_sample: 0,
    }))
}

pub(super) fn gather_track_info(file_path: &str) -> TrackInfo {
    let probed = match get_probe_result_from_string(file_path) {
        Ok(probed) => probed,
        Err(_) => return fallback_track_info(file_path),
    };

    let tracks = probed.format.tracks();
    if tracks.is_empty() {
        return fallback_track_info(file_path);
    }
    let mut track_infos: Vec<TrackInfo> = Vec::new();
    for track in tracks {
        let track_info = get_track_info(track);
        track_infos.push(track_info);
    }

    let mut info = match reduce_track_infos(track_infos) {
        Ok(info) => info,
        Err(e) => {
            warn!("incompatible track formats in '{}': {}", file_path, e);
            return fallback_track_info(file_path);
        }
    };
    if info.bits_per_sample == 0 {
        let decoded_bits = bits_from_decode(file_path);
        if decoded_bits > 0 {
            info.bits_per_sample = decoded_bits;
        }
    }
    if info.sample_rate == 0 && info.channel_count == 0 && info.bits_per_sample == 0 {
        return fallback_track_info(file_path);
    }
    info
}

pub(super) fn gather_track_info_from_file_paths(file_paths: Vec<String>) -> TrackInfo {
    let mut track_infos: Vec<TrackInfo> = Vec::new();

    for file_path in file_paths {
        debug!("File path: {:?}", file_path);
        let track_info = gather_track_info(&file_path);
        track_infos.push(track_info);
    }

    match reduce_track_infos(track_infos) {
        Ok(info) => info,
        Err(e) => {
            warn!("incompatible track formats across file set: {}", e);
            TrackInfo {
                sample_rate: 0,
                channel_count: 0,
                bits_per_sample: 0,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduce_track_infos_errors_on_mismatched_sample_rates() {
        let infos = vec![
            TrackInfo {
                sample_rate: 44100,
                channel_count: 2,
                bits_per_sample: 16,
            },
            TrackInfo {
                sample_rate: 48000,
                channel_count: 2,
                bits_per_sample: 16,
            },
        ];
        let result = reduce_track_infos(infos);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("sample rate"),
            "expected 'sample rate' in: {}",
            msg
        );
    }

    #[test]
    fn reduce_track_infos_errors_on_mismatched_channel_counts() {
        let infos = vec![
            TrackInfo {
                sample_rate: 44100,
                channel_count: 1,
                bits_per_sample: 16,
            },
            TrackInfo {
                sample_rate: 44100,
                channel_count: 2,
                bits_per_sample: 16,
            },
        ];
        let result = reduce_track_infos(infos);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("channel"), "expected 'channel' in: {}", msg);
    }

    #[test]
    fn reduce_track_infos_errors_on_mismatched_bits_per_sample() {
        let infos = vec![
            TrackInfo {
                sample_rate: 44100,
                channel_count: 2,
                bits_per_sample: 16,
            },
            TrackInfo {
                sample_rate: 44100,
                channel_count: 2,
                bits_per_sample: 24,
            },
        ];
        let result = reduce_track_infos(infos);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("bits per sample"),
            "expected 'bits per sample' in: {}",
            msg
        );
    }

    #[test]
    fn reduce_track_infos_succeeds_with_compatible_tracks() {
        let infos = vec![
            TrackInfo {
                sample_rate: 44100,
                channel_count: 2,
                bits_per_sample: 16,
            },
            TrackInfo {
                sample_rate: 44100,
                channel_count: 2,
                bits_per_sample: 16,
            },
        ];
        let result = reduce_track_infos(infos);
        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.sample_rate, 44100);
        assert_eq!(info.channel_count, 2);
        assert_eq!(info.bits_per_sample, 16);
    }

    #[test]
    fn reduce_track_infos_fills_in_zero_fields_from_other_tracks() {
        let infos = vec![
            TrackInfo {
                sample_rate: 0,
                channel_count: 2,
                bits_per_sample: 16,
            },
            TrackInfo {
                sample_rate: 44100,
                channel_count: 2,
                bits_per_sample: 16,
            },
        ];
        let result = reduce_track_infos(infos);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().sample_rate, 44100);
    }

    #[test]
    fn reduce_track_infos_empty_returns_zeros() {
        let result = reduce_track_infos(vec![]);
        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.sample_rate, 0);
        assert_eq!(info.channel_count, 0);
        assert_eq!(info.bits_per_sample, 0);
    }
}
