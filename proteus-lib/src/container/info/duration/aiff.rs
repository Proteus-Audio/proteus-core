use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

use symphonia::core::formats::Track;

use super::{single_track_detail, DurationSourceKind};

pub(super) fn probe(file_path: &str, tracks: &[Track]) -> Option<Vec<super::DurationDetail>> {
    let ext = super::extension_lc(file_path)?;
    if !matches!(ext.as_str(), "aiff" | "aif" | "aifc" | "aaif") {
        return None;
    }

    let seconds = probe_seconds(file_path)?;
    single_track_detail(seconds, DurationSourceKind::Structural, true, tracks)
}

pub(super) fn probe_standalone(file_path: &str) -> Option<Vec<super::DurationDetail>> {
    let ext = super::extension_lc(file_path)?;
    if !matches!(ext.as_str(), "aiff" | "aif" | "aifc" | "aaif") {
        return None;
    }
    super::standalone_detail(
        probe_seconds(file_path)?,
        DurationSourceKind::Structural,
        true,
    )
}

fn probe_seconds(file_path: &str) -> Option<f64> {
    let mut file = File::open(file_path).ok()?;
    let info = parse_aiff(&mut file)?;
    Some(info.sample_frames as f64 / info.sample_rate)
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct AiffDurationInfo {
    sample_rate: f64,
    sample_frames: u32,
}

fn parse_aiff<R: Read + Seek>(reader: &mut R) -> Option<AiffDurationInfo> {
    let mut header = [0u8; 12];
    reader.read_exact(&mut header).ok()?;
    if &header[0..4] != b"FORM" {
        return None;
    }
    if &header[8..12] != b"AIFF" && &header[8..12] != b"AIFC" {
        return None;
    }

    loop {
        let mut chunk = [0u8; 8];
        if reader.read_exact(&mut chunk).is_err() {
            return None;
        }
        let chunk_size = u32::from_be_bytes(chunk[4..8].try_into().ok()?) as u64;
        if &chunk[0..4] == b"COMM" {
            if chunk_size < 18 {
                return None;
            }
            let mut comm = vec![0u8; chunk_size as usize];
            reader.read_exact(&mut comm).ok()?;
            let sample_frames = u32::from_be_bytes(comm[2..6].try_into().ok()?);
            let sample_rate = extended_80_to_f64(comm[8..18].try_into().ok()?);
            if sample_rate <= 0.0 || sample_frames == 0 {
                return None;
            }
            return Some(AiffDurationInfo {
                sample_rate,
                sample_frames,
            });
        }

        reader
            .seek(SeekFrom::Current((chunk_size + (chunk_size % 2)) as i64))
            .ok()?;
    }
}

fn extended_80_to_f64(bytes: [u8; 10]) -> f64 {
    let sign = (bytes[0] & 0x80) != 0;
    let exponent = (((bytes[0] & 0x7F) as u16) << 8) | bytes[1] as u16;
    let mut mantissa: u64 = 0;
    for byte in &bytes[2..] {
        mantissa = (mantissa << 8) | *byte as u64;
    }

    if exponent == 0 && mantissa == 0 {
        return 0.0;
    }
    if exponent == 0x7FFF {
        return f64::NAN;
    }

    let value = 2f64.powi(exponent as i32 - 16383) * (mantissa as f64 / (1u64 << 63) as f64);
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
    fn extended_rate_converts_44100() {
        let bytes = [0x40, 0x0e, 0xac, 0x44, 0, 0, 0, 0, 0, 0];

        assert!((extended_80_to_f64(bytes) - 44_100.0).abs() < 0.001);
    }
}
