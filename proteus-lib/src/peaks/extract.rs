use log::warn;
use symphonia::core::audio::{AudioBufferRef, Channels, Signal};
use symphonia::core::errors::Error;

use crate::tools::tools::open_file;

use super::{PeakWindow, PeaksData, PeaksError};

fn convert_signed_24bit_to_f32(sample: i32) -> f32 {
    let shifted_sample = sample << 8 >> 8;
    shifted_sample as f32 / 2f32.powi(23)
}

fn convert_unsigned_24bit_to_f32(sample: u32) -> f32 {
    let shifted_sample = sample as i32 - 2i32.pow(23);
    shifted_sample as f32 / 2f32.powi(23)
}

fn convert_signed_16bit_to_f32(sample: i16) -> f32 {
    sample as f32 / 2f32.powi(15)
}

fn convert_unsigned_16bit_to_f32(sample: u16) -> f32 {
    let shifted_sample = sample as i16 - 2i16.pow(15);
    shifted_sample as f32 / 2f32.powi(15)
}

fn convert_signed_8bit_to_f32(sample: i8) -> f32 {
    sample as f32 / 2f32.powi(7)
}

fn convert_unsigned_8bit_to_f32(sample: u8) -> f32 {
    let shifted_sample = sample as i16 - 2i16.pow(7);
    shifted_sample as f32 / 2f32.powi(7)
}

fn convert_signed_32bit_to_f32(sample: i32) -> f32 {
    sample as f32 / 2f32.powi(31)
}

fn convert_unsigned_32bit_to_f32(sample: u32) -> f32 {
    let shifted_sample = sample as i64 - 2i64.pow(31);
    shifted_sample as f32 / 2f32.powi(31)
}

#[derive(Debug)]
struct ChannelAccumulator {
    current_max: f32,
    current_min: f32,
    count: usize,
    peaks: Vec<PeakWindow>,
}

impl ChannelAccumulator {
    fn new() -> Self {
        Self {
            current_max: f32::MIN,
            current_min: f32::MAX,
            count: 0,
            peaks: Vec::new(),
        }
    }

    fn push(&mut self, sample: f32, window_size: usize) {
        self.current_max = self.current_max.max(sample);
        self.current_min = self.current_min.min(sample);
        self.count += 1;

        if self.count == window_size {
            self.peaks.push(PeakWindow {
                max: self.current_max,
                min: self.current_min,
            });
            self.reset_window();
        }
    }

    fn flush_partial(&mut self) {
        if self.count > 0 {
            self.peaks.push(PeakWindow {
                max: self.current_max,
                min: self.current_min,
            });
            self.reset_window();
        }
    }

    fn reset_window(&mut self) {
        self.current_max = f32::MIN;
        self.current_min = f32::MAX;
        self.count = 0;
    }
}

pub(super) fn extract_peaks_from_audio(
    file_path: &str,
    limited: bool,
) -> Result<PeaksData, PeaksError> {
    let (mut decoder, mut format) = open_file(file_path);

    let track = format
        .tracks()
        .first()
        .ok_or_else(|| PeaksError::Decode("no audio tracks found".to_string()))?;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| PeaksError::Decode("missing sample rate in codec params".to_string()))?;
    let window_size = (sample_rate / 100).max(1) as usize;

    let channels = if limited {
        1
    } else {
        track
            .codec_params
            .channels
            .unwrap_or(Channels::FRONT_CENTRE)
            .iter()
            .count()
            .max(1)
    };

    let track_id = track.id;
    let mut accumulators = (0..channels)
        .map(|_| ChannelAccumulator::new())
        .collect::<Vec<_>>();

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(err)) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => {
                return Err(PeaksError::Decode(
                    "decoder reset required while extracting peaks".to_string(),
                ));
            }
            Err(err) => return Err(PeaksError::Decode(err.to_string())),
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                let decoded_channels = decoded.spec().channels.count();
                let channel_limit = channels.min(decoded_channels);

                match decoded {
                    AudioBufferRef::U8(buf) => process_channels(
                        channel_limit,
                        &mut accumulators,
                        window_size,
                        |channel, push| {
                            for &sample in buf.chan(channel) {
                                push(convert_unsigned_8bit_to_f32(sample));
                            }
                        },
                    ),
                    AudioBufferRef::S8(buf) => process_channels(
                        channel_limit,
                        &mut accumulators,
                        window_size,
                        |channel, push| {
                            for &sample in buf.chan(channel) {
                                push(convert_signed_8bit_to_f32(sample));
                            }
                        },
                    ),
                    AudioBufferRef::U16(buf) => process_channels(
                        channel_limit,
                        &mut accumulators,
                        window_size,
                        |channel, push| {
                            for &sample in buf.chan(channel) {
                                push(convert_unsigned_16bit_to_f32(sample));
                            }
                        },
                    ),
                    AudioBufferRef::S16(buf) => process_channels(
                        channel_limit,
                        &mut accumulators,
                        window_size,
                        |channel, push| {
                            for &sample in buf.chan(channel) {
                                push(convert_signed_16bit_to_f32(sample));
                            }
                        },
                    ),
                    AudioBufferRef::U24(buf) => process_channels(
                        channel_limit,
                        &mut accumulators,
                        window_size,
                        |channel, push| {
                            for sample in buf.chan(channel) {
                                push(convert_unsigned_24bit_to_f32(sample.0));
                            }
                        },
                    ),
                    AudioBufferRef::S24(buf) => process_channels(
                        channel_limit,
                        &mut accumulators,
                        window_size,
                        |channel, push| {
                            for sample in buf.chan(channel) {
                                push(convert_signed_24bit_to_f32(sample.0));
                            }
                        },
                    ),
                    AudioBufferRef::U32(buf) => process_channels(
                        channel_limit,
                        &mut accumulators,
                        window_size,
                        |channel, push| {
                            for &sample in buf.chan(channel) {
                                push(convert_unsigned_32bit_to_f32(sample));
                            }
                        },
                    ),
                    AudioBufferRef::S32(buf) => process_channels(
                        channel_limit,
                        &mut accumulators,
                        window_size,
                        |channel, push| {
                            for &sample in buf.chan(channel) {
                                push(convert_signed_32bit_to_f32(sample));
                            }
                        },
                    ),
                    AudioBufferRef::F32(buf) => process_channels(
                        channel_limit,
                        &mut accumulators,
                        window_size,
                        |channel, push| {
                            for &sample in buf.chan(channel) {
                                push(sample);
                            }
                        },
                    ),
                    AudioBufferRef::F64(buf) => process_channels(
                        channel_limit,
                        &mut accumulators,
                        window_size,
                        |channel, push| {
                            for &sample in buf.chan(channel) {
                                push(sample as f32);
                            }
                        },
                    ),
                }
            }
            Err(Error::DecodeError(err)) => {
                warn!("decode error: {}", err);
            }
            Err(err) => return Err(PeaksError::Decode(err.to_string())),
        }
    }

    let channels = accumulators
        .iter_mut()
        .map(|acc| {
            acc.flush_partial();
            std::mem::take(&mut acc.peaks)
        })
        .collect();

    Ok(PeaksData {
        sample_rate,
        window_size: window_size as u32,
        channels,
    })
}

fn process_channels<F>(
    channels: usize,
    accumulators: &mut [ChannelAccumulator],
    window_size: usize,
    mut each_channel: F,
) where
    F: FnMut(usize, &mut dyn FnMut(f32)),
{
    for channel in 0..channels {
        let mut push_sample = |sample: f32| {
            accumulators[channel].push(sample, window_size);
        };
        each_channel(channel, &mut push_sample);
    }
}
