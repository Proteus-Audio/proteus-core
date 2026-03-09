//! Shared sample-decoding helpers used across peaks, track buffering, and mix decode.

use symphonia::core::audio::{AudioBufferRef, Signal};

/// Convert an unsigned 8-bit sample to `f32`.
pub fn convert_unsigned_8bit_to_f32(sample: u8) -> f32 {
    let shifted_sample = sample as i16 - 2i16.pow(7);
    shifted_sample as f32 / 2f32.powi(7)
}

/// Convert a signed 8-bit sample to `f32`.
pub fn convert_signed_8bit_to_f32(sample: i8) -> f32 {
    sample as f32 / 2f32.powi(7)
}

/// Convert a signed 24-bit sample stored in an `i32` to `f32`.
pub fn convert_signed_24bit_to_f32(sample: i32) -> f32 {
    let shifted_sample = sample << 8 >> 8;
    shifted_sample as f32 / 2f32.powi(23)
}

/// Convert an unsigned 24-bit sample stored in a `u32` to `f32`.
pub fn convert_unsigned_24bit_to_f32(sample: u32) -> f32 {
    let shifted_sample = sample as i32 - 2i32.pow(23);
    shifted_sample as f32 / 2f32.powi(23)
}

/// Convert a signed 16-bit sample to `f32`.
pub fn convert_signed_16bit_to_f32(sample: i16) -> f32 {
    sample as f32 / 2f32.powi(15)
}

/// Convert an unsigned 16-bit sample to `f32`.
pub fn convert_unsigned_16bit_to_f32(sample: u16) -> f32 {
    let shifted_sample = sample as i32 - 2_i32.pow(15);
    shifted_sample as f32 / 2f32.powi(15)
}

/// Convert a signed 32-bit sample to `f32`.
pub fn convert_signed_32bit_to_f32(sample: i32) -> f32 {
    sample as f32 / 2f32.powi(31)
}

/// Convert an unsigned 32-bit sample to `f32`.
pub fn convert_unsigned_32bit_to_f32(sample: u32) -> f32 {
    let shifted_sample = sample as i64 - 2i64.pow(31);
    shifted_sample as f32 / 2f32.powi(31)
}

/// Iterate through decoded samples for a single channel and map them to `f32`.
pub fn for_each_channel_sample(
    decoded: &AudioBufferRef<'_>,
    channel: usize,
    mut on_sample: impl FnMut(f32),
) {
    match decoded {
        AudioBufferRef::U8(buf) => {
            for &sample in buf.chan(channel) {
                on_sample(convert_unsigned_8bit_to_f32(sample));
            }
        }
        AudioBufferRef::S8(buf) => {
            for &sample in buf.chan(channel) {
                on_sample(convert_signed_8bit_to_f32(sample));
            }
        }
        AudioBufferRef::U16(buf) => {
            for &sample in buf.chan(channel) {
                on_sample(convert_unsigned_16bit_to_f32(sample));
            }
        }
        AudioBufferRef::S16(buf) => {
            for &sample in buf.chan(channel) {
                on_sample(convert_signed_16bit_to_f32(sample));
            }
        }
        AudioBufferRef::U24(buf) => {
            for sample in buf.chan(channel) {
                on_sample(convert_unsigned_24bit_to_f32(sample.0));
            }
        }
        AudioBufferRef::S24(buf) => {
            for sample in buf.chan(channel) {
                on_sample(convert_signed_24bit_to_f32(sample.0));
            }
        }
        AudioBufferRef::U32(buf) => {
            for &sample in buf.chan(channel) {
                on_sample(convert_unsigned_32bit_to_f32(sample));
            }
        }
        AudioBufferRef::S32(buf) => {
            for &sample in buf.chan(channel) {
                on_sample(convert_signed_32bit_to_f32(sample));
            }
        }
        AudioBufferRef::F32(buf) => {
            for &sample in buf.chan(channel) {
                on_sample(sample);
            }
        }
        AudioBufferRef::F64(buf) => {
            for &sample in buf.chan(channel) {
                on_sample(sample as f32);
            }
        }
    }
}

/// Extract samples for a single channel from a decoded packet.
pub fn process_channel(decoded: AudioBufferRef<'_>, channel: usize) -> Vec<f32> {
    let mut samples = Vec::new();
    for_each_channel_sample(&decoded, channel, |sample| samples.push(sample));
    samples
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_and_unsigned_conversions_are_centered() {
        assert_eq!(convert_unsigned_8bit_to_f32(128), 0.0);
        assert_eq!(convert_unsigned_16bit_to_f32(32768), 0.0);
        assert_eq!(convert_unsigned_32bit_to_f32(2_u32.pow(31)), 0.0);
    }

    #[test]
    fn signed_conversions_scale_as_expected() {
        assert!((convert_signed_8bit_to_f32(64) - 0.5).abs() < 1e-6);
        assert!((convert_signed_16bit_to_f32(16384) - 0.5).abs() < 1e-6);
        assert!((convert_signed_24bit_to_f32(4_194_304) - 0.5).abs() < 1e-6);
    }
}
