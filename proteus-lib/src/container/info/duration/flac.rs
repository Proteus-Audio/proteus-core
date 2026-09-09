use std::{fs::File, io::Read};

use symphonia::core::formats::Track;

use super::{single_track_detail, DurationSourceKind};

pub(super) fn probe(file_path: &str, tracks: &[Track]) -> Option<Vec<super::DurationDetail>> {
    if super::extension_lc(file_path).as_deref() != Some("flac") {
        return None;
    }

    let seconds = probe_seconds(file_path)?;
    single_track_detail(seconds, DurationSourceKind::Structural, true, tracks)
}

pub(super) fn probe_standalone(file_path: &str) -> Option<Vec<super::DurationDetail>> {
    if super::extension_lc(file_path).as_deref() != Some("flac") {
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
    let info = parse_streaminfo(&mut file)?;
    Some(info.total_samples as f64 / info.sample_rate as f64)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StreamInfo {
    sample_rate: u32,
    total_samples: u64,
}

fn parse_streaminfo<R: Read>(reader: &mut R) -> Option<StreamInfo> {
    let mut marker = [0u8; 4];
    reader.read_exact(&mut marker).ok()?;
    if &marker != b"fLaC" {
        return None;
    }

    loop {
        let mut header = [0u8; 4];
        reader.read_exact(&mut header).ok()?;
        let is_last = (header[0] & 0x80) != 0;
        let block_type = header[0] & 0x7f;
        let len = ((header[1] as usize) << 16) | ((header[2] as usize) << 8) | header[3] as usize;

        if block_type == 0 {
            if len < 34 {
                return None;
            }
            let mut data = vec![0u8; len];
            reader.read_exact(&mut data).ok()?;
            let packed = u64::from_be_bytes(data[10..18].try_into().ok()?);
            let sample_rate = ((packed >> 44) & 0x0f_ffff) as u32;
            let total_samples = packed & 0x0f_ffff_ffff;
            if sample_rate == 0 || total_samples == 0 {
                return None;
            }
            return Some(StreamInfo {
                sample_rate,
                total_samples,
            });
        }

        std::io::copy(&mut reader.take(len as u64), &mut std::io::sink()).ok()?;
        if is_last {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_streaminfo_total_samples() {
        let mut data = Vec::new();
        data.extend_from_slice(b"fLaC");
        data.extend_from_slice(&[0x80, 0x00, 0x00, 34]);
        let mut streaminfo = [0u8; 34];
        let packed = ((44_100u64) << 44) | 1_771_776u64;
        streaminfo[10..18].copy_from_slice(&packed.to_be_bytes());
        data.extend_from_slice(&streaminfo);

        let info = parse_streaminfo(&mut &data[..]).expect("streaminfo");

        assert_eq!(info.sample_rate, 44_100);
        assert_eq!(info.total_samples, 1_771_776);
    }
}
