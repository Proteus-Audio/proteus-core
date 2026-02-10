//! Sample format conversion helpers for track decoding.

use symphonia::core::audio::{AudioBufferRef, Signal};

/// Convert a signed 24-bit sample stored in an `i32` to `f32`.
pub fn convert_signed_24bit_to_f32(sample: i32) -> f32 {
    // Assuming the 24-bit sample is the least significant bits of a 32-bit integer.
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
    let shifted_sample = sample as i16 - 2i16.pow(15);
    shifted_sample as f32 / 2f32.powi(15)
}

/// Convert a signed 32-bit sample to `f32`.
pub fn convert_signed_32bit_to_f32(sample: i32) -> f32 {
    sample as f32 / 2f32.powi(31)
}

/// Extract samples for a single channel from a decoded packet.
pub fn process_channel(decoded: AudioBufferRef<'_>, channel: usize) -> Vec<f32> {
    match decoded {
        AudioBufferRef::U16(buf) => buf
            .chan(channel)
            .to_vec()
            .into_iter()
            .map(convert_unsigned_16bit_to_f32)
            .collect(),
        AudioBufferRef::S16(buf) => buf
            .chan(channel)
            .to_vec()
            .into_iter()
            .map(convert_signed_16bit_to_f32)
            .collect(),
        AudioBufferRef::U24(buf) => buf
            .chan(channel)
            .to_vec()
            .into_iter()
            .map(|s| convert_unsigned_24bit_to_f32(s.0))
            .collect(),
        AudioBufferRef::S24(buf) => buf
            .chan(channel)
            .to_vec()
            .into_iter()
            .map(|s| convert_signed_24bit_to_f32(s.0))
            .collect(),
        AudioBufferRef::S32(buf) => buf
            .chan(channel)
            .to_vec()
            .into_iter()
            .map(convert_signed_32bit_to_f32)
            .collect(),
        AudioBufferRef::F32(buf) => buf.chan(channel).to_vec().into_iter().collect(),
        _ => {
            // Repeat for the different sample formats as needed.
            unimplemented!();
        }
    }
}
