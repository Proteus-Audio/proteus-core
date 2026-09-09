use std::ffi::OsStr;
use std::fs::File;
use std::num::{NonZeroU32, NonZeroU8};
use std::path::Path;

use proteus_mux::VorbisHeaders;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use vorbis_rs::{VorbisBitrateManagementStrategy, VorbisEncoderBuilder};

use super::ogg::{read_le_u32, read_ogg_packets, time_packets, TimedPacket};
use super::{MuxError, Result};

#[derive(Debug)]
pub(super) struct OggVorbisInput {
    pub(super) sample_rate: u32,
    pub(super) channels: u8,
    pub(super) headers: VorbisHeaders,
    pub(super) packets: Vec<TimedPacket>,
}

pub(super) fn transcode_to_vorbis(path: &Path, stream_serial: i32) -> Result<OggVorbisInput> {
    let encoded_ogg = encode_file_to_ogg_vorbis(path, stream_serial)?;
    let packets = read_ogg_packets(&encoded_ogg, &path.display().to_string())?;
    if packets.len() < 4 {
        return Err(MuxError::InvalidInput(format!(
            "{} did not produce Vorbis headers and audio packets",
            path.display()
        )));
    }

    let identification = packets[0].data.clone();
    let comment = packets[1].data.clone();
    let setup = packets[2].data.clone();
    let channels = *identification.get(11).ok_or_else(|| {
        MuxError::InvalidInput(format!(
            "{} has a truncated Vorbis identification header",
            path.display()
        ))
    })?;
    let sample_rate = read_le_u32(&identification, 12).ok_or_else(|| {
        MuxError::InvalidInput(format!(
            "{} has a truncated Vorbis identification header",
            path.display()
        ))
    })?;
    let headers = VorbisHeaders::new(identification, comment, setup)?;
    let timed_packets = time_packets(&packets[3..], sample_rate);

    Ok(OggVorbisInput {
        sample_rate,
        channels,
        headers,
        packets: timed_packets,
    })
}

fn encode_file_to_ogg_vorbis(path: &Path, stream_serial: i32) -> Result<Vec<u8>> {
    let source = Box::new(File::open(path)?);
    let media_source = MediaSourceStream::new(source, Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(OsStr::to_str) {
        hint.with_extension(extension);
    }

    let probed = symphonia::default::get_probe().format(
        &hint,
        media_source,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| {
            MuxError::InvalidInput(format!("{} has no supported audio track", path.display()))
        })?;
    let track_id = track.id;
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let mut encoder = None;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                return Err(MuxError::InvalidInput(format!(
                    "{} changed streams while decoding; chained streams are not supported",
                    path.display()
                )));
            }
            Err(error) => return Err(error.into()),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) | Err(SymphoniaError::IoError(_)) => continue,
            Err(error) => return Err(error.into()),
        };

        let spec = *decoded.spec();
        let channels = spec.channels.count();
        if channels == 0 || channels > 8 {
            return Err(MuxError::InvalidInput(format!(
                "{} has {channels} channels; muxing supports 1 through 8",
                path.display()
            )));
        }
        if spec.rate == 0 {
            return Err(MuxError::InvalidInput(format!(
                "{} has an invalid sample rate",
                path.display()
            )));
        }

        if encoder.is_none() {
            let mut builder = VorbisEncoderBuilder::new_with_serial(
                NonZeroU32::new(spec.rate).ok_or_else(|| {
                    MuxError::InvalidInput("decoded audio has zero sample rate".to_string())
                })?,
                NonZeroU8::new(u8::try_from(channels)?).ok_or_else(|| {
                    MuxError::InvalidInput("decoded audio has zero channels".to_string())
                })?,
                Vec::new(),
                stream_serial,
            );
            builder.bitrate_management_strategy(VorbisBitrateManagementStrategy::QualityVbr {
                target_quality: 0.4,
            });
            encoder = Some(builder.build()?);
        }

        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
        buffer.copy_interleaved_ref(decoded);
        let planar = interleaved_to_planar(buffer.samples(), channels);
        if !planar.is_empty() && !planar[0].is_empty() {
            encoder
                .as_mut()
                .expect("encoder initialized above")
                .encode_audio_block(&planar)?;
        }
    }

    let encoder = encoder.ok_or_else(|| {
        MuxError::InvalidInput(format!("{} did not decode any audio", path.display()))
    })?;
    Ok(encoder.finish()?)
}

fn interleaved_to_planar(samples: &[f32], channels: usize) -> Vec<Vec<f32>> {
    let frames = samples.len() / channels;
    let mut planar = vec![Vec::with_capacity(frames); channels];

    for frame in samples.chunks_exact(channels) {
        for (channel, sample) in frame.iter().enumerate() {
            planar[channel].push(*sample);
        }
    }

    planar
}
