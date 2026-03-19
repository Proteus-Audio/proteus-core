//! AIFF format parsing and fallback metadata extraction.

use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use super::TrackInfo;

#[derive(Debug, Clone, Copy)]
struct AiffInfo {
    channels: u16,
    sample_rate: f64,
    bits_per_sample: u16,
    sample_frames: u32,
}

pub(super) fn fallback_track_info(file_path: &str) -> TrackInfo {
    if let Some(info) = parse_aiff_info(file_path) {
        return TrackInfo {
            sample_rate: info.sample_rate.round() as u32,
            channel_count: info.channels as u32,
            bits_per_sample: info.bits_per_sample as u32,
        };
    }

    TrackInfo {
        sample_rate: 0,
        channel_count: 0,
        bits_per_sample: 0,
    }
}

pub(super) fn fallback_durations(file_path: &str) -> HashMap<u32, f64> {
    if let Some(info) = parse_aiff_info(file_path) {
        let duration = if info.sample_rate > 0.0 {
            info.sample_frames as f64 / info.sample_rate
        } else {
            0.0
        };
        let mut map = HashMap::new();
        map.insert(0, duration);
        return map;
    }

    HashMap::new()
}

fn parse_aiff_info(file_path: &str) -> Option<AiffInfo> {
    let path = Path::new(file_path);
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())?
        .to_lowercase();
    if ext != "aiff" && ext != "aif" && ext != "aifc" && ext != "aaif" {
        return None;
    }

    let mut file = File::open(path).ok()?;
    let mut header = [0u8; 12];
    file.read_exact(&mut header).ok()?;
    if &header[0..4] != b"FORM" {
        return None;
    }
    let form_type = &header[8..12];
    if form_type != b"AIFF" && form_type != b"AIFC" {
        return None;
    }

    loop {
        let mut chunk_header = [0u8; 8];
        if file.read_exact(&mut chunk_header).is_err() {
            break;
        }
        let chunk_id = &chunk_header[0..4];
        let chunk_size = u32::from_be_bytes([
            chunk_header[4],
            chunk_header[5],
            chunk_header[6],
            chunk_header[7],
        ]) as u64;

        if chunk_id == b"COMM" {
            if chunk_size < 18 {
                return None;
            }
            let mut comm = vec![0u8; chunk_size as usize];
            file.read_exact(&mut comm).ok()?;
            let channels = u16::from_be_bytes([comm[0], comm[1]]);
            let sample_frames = u32::from_be_bytes([comm[2], comm[3], comm[4], comm[5]]);
            let bits_per_sample = u16::from_be_bytes([comm[6], comm[7]]);
            let mut rate_bytes = [0u8; 10];
            rate_bytes.copy_from_slice(&comm[8..18]);
            let sample_rate = extended_80_to_f64(rate_bytes);

            return Some(AiffInfo {
                channels,
                sample_rate,
                bits_per_sample,
                sample_frames,
            });
        }

        let skip = chunk_size + (chunk_size % 2);
        if file.seek(SeekFrom::Current(skip as i64)).is_err() {
            break;
        }
    }

    None
}

fn extended_80_to_f64(bytes: [u8; 10]) -> f64 {
    let sign = (bytes[0] & 0x80) != 0;
    let exponent = (((bytes[0] & 0x7F) as u16) << 8) | bytes[1] as u16;
    let mut mantissa: u64 = 0;
    for i in 0..8 {
        mantissa = (mantissa << 8) | bytes[2 + i] as u64;
    }

    if exponent == 0 && mantissa == 0 {
        return 0.0;
    }
    if exponent == 0x7FFF {
        return f64::NAN;
    }

    let exp = exponent as i32 - 16383;
    let fraction = mantissa as f64 / (1u64 << 63) as f64;
    let value = 2f64.powi(exp) * fraction;
    if sign {
        -value
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extended_80_to_f64_handles_zero_and_negative_values() {
        assert_eq!(extended_80_to_f64([0; 10]), 0.0);
        let negative_one = [0xBF, 0xFF, 0x80, 0, 0, 0, 0, 0, 0, 0];
        assert!((extended_80_to_f64(negative_one) + 1.0).abs() < 1e-6);
    }
}
