use std::fmt;
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek};
use std::path::Path;

use log::warn;
use matroska::Matroska;
use rodio::{Decoder, Source};

#[derive(Debug, Clone)]
pub struct ImpulseResponse {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f32>>,
}

impl ImpulseResponse {
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    pub fn channel_for_output(&self, index: usize) -> &[f32] {
        if self.channels.is_empty() {
            return &[];
        }

        if self.channels.len() == 1 {
            return &self.channels[0];
        }

        let channel_index = index % self.channels.len();
        &self.channels[channel_index]
    }
}

#[derive(Debug)]
pub enum ImpulseResponseError {
    Io(std::io::Error),
    Matroska(matroska::Error),
    Decode(rodio::decoder::DecoderError),
    AttachmentNotFound(String),
    InvalidChannels,
}

impl fmt::Display for ImpulseResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "failed to read impulse response: {}", err),
            Self::Matroska(err) => write!(f, "failed to read prot container: {}", err),
            Self::Decode(err) => write!(f, "failed to decode impulse response: {}", err),
            Self::AttachmentNotFound(name) => {
                write!(f, "impulse response attachment not found: {}", name)
            }
            Self::InvalidChannels => write!(f, "impulse response has invalid channel count"),
        }
    }
}

impl std::error::Error for ImpulseResponseError {}

impl From<std::io::Error> for ImpulseResponseError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<rodio::decoder::DecoderError> for ImpulseResponseError {
    fn from(err: rodio::decoder::DecoderError) -> Self {
        Self::Decode(err)
    }
}

impl From<matroska::Error> for ImpulseResponseError {
    fn from(err: matroska::Error) -> Self {
        Self::Matroska(err)
    }
}

pub fn load_impulse_response_from_file(
    path: impl AsRef<Path>,
) -> Result<ImpulseResponse, ImpulseResponseError> {
    load_impulse_response_from_file_with_tail(path, Some(-60.0))
}

pub fn load_impulse_response_from_file_with_tail(
    path: impl AsRef<Path>,
    tail_db: Option<f32>,
) -> Result<ImpulseResponse, ImpulseResponseError> {
    let file = File::open(path)?;
    decode_impulse_response(BufReader::new(file), tail_db)
}

pub fn load_impulse_response_from_bytes(
    bytes: &[u8],
) -> Result<ImpulseResponse, ImpulseResponseError> {
    load_impulse_response_from_bytes_with_tail(bytes, Some(-60.0))
}

pub fn load_impulse_response_from_bytes_with_tail(
    bytes: &[u8],
    tail_db: Option<f32>,
) -> Result<ImpulseResponse, ImpulseResponseError> {
    decode_impulse_response(BufReader::new(Cursor::new(bytes.to_vec())), tail_db)
}

pub fn load_impulse_response_from_prot_attachment(
    prot_path: impl AsRef<Path>,
    attachment_name: &str,
) -> Result<ImpulseResponse, ImpulseResponseError> {
    load_impulse_response_from_prot_attachment_with_tail(prot_path, attachment_name, Some(-60.0))
}

pub fn load_impulse_response_from_prot_attachment_with_tail(
    prot_path: impl AsRef<Path>,
    attachment_name: &str,
    tail_db: Option<f32>,
) -> Result<ImpulseResponse, ImpulseResponseError> {
    let file = File::open(prot_path)?;
    let mka: Matroska = Matroska::open(file)?;

    let attachment = mka
        .attachments
        .iter()
        .find(|attachment| attachment.name == attachment_name)
        .ok_or_else(|| ImpulseResponseError::AttachmentNotFound(attachment_name.to_string()))?;

    load_impulse_response_from_bytes_with_tail(&attachment.data, tail_db)
}

fn decode_impulse_response<R>(
    reader: R,
    tail_db: Option<f32>,
) -> Result<ImpulseResponse, ImpulseResponseError>
where
    R: Read + Seek + Send + Sync + 'static,
{
    let source = Decoder::new(reader)?;
    let channels = source.channels() as usize;
    if channels == 0 {
        return Err(ImpulseResponseError::InvalidChannels);
    }

    let sample_rate = source.sample_rate();
    let mut channel_samples = vec![Vec::new(); channels];

    for (index, sample) in source.enumerate() {
        channel_samples[index % channels].push(sample as f32);
    }

    let mut max_abs = 0.0_f32;
    for channel in &channel_samples {
        for sample in channel {
            let abs = sample.abs();
            if abs > max_abs {
                max_abs = abs;
            }
        }
    }

    if max_abs > 0.0 {
        let scale = 1.0 / max_abs;
        for channel in &mut channel_samples {
            for sample in channel {
                *sample *= scale;
            }
        }
    }

    if let Some(tail_db) = tail_db {
        if tail_db.is_finite() {
            trim_impulse_response_tail(&mut channel_samples, tail_db);
        }
    }

    if channel_samples.iter().any(|channel| channel.is_empty()) {
        warn!("Impulse response includes empty channels; results may be silent.");
    }

    Ok(ImpulseResponse {
        sample_rate,
        channels: channel_samples,
    })
}

fn trim_impulse_response_tail(channels: &mut [Vec<f32>], tail_db: f32) {
    if channels.is_empty() {
        return;
    }

    let threshold = 10.0_f32.powf(tail_db / 20.0).abs();
    if threshold <= 0.0 {
        return;
    }

    let mut last_index = 0usize;
    for (channel_index, channel) in channels.iter().enumerate() {
        if channel.is_empty() {
            continue;
        }
        let mut channel_last = None;
        for (index, sample) in channel.iter().enumerate() {
            if sample.abs() >= threshold {
                channel_last = Some(index);
            }
        }
        if let Some(channel_last) = channel_last {
            if channel_index == 0 || channel_last > last_index {
                last_index = channel_last;
            }
        }
    }

    let keep_len = (last_index + 1).max(1);
    for channel in channels.iter_mut() {
        if channel.len() > keep_len {
            channel.truncate(keep_len);
        }
    }
}
