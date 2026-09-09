use std::{fs::File, io::Read};

use symphonia::core::formats::Track;

use super::{single_track_detail, DurationSourceKind};

const ADTS_HEADER_LEN: usize = 7;
const ADTS_SAMPLES_PER_FRAME: u64 = 1024;
const SAMPLE_RATES: [u32; 16] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350, 0, 0, 0,
];

pub(super) fn probe(file_path: &str, tracks: &[Track]) -> Option<Vec<super::DurationDetail>> {
    let ext = super::extension_lc(file_path)?;
    if !matches!(ext.as_str(), "aac" | "adts") {
        return None;
    }

    let seconds = probe_seconds(file_path)?;
    single_track_detail(seconds, DurationSourceKind::HeaderScan, true, tracks)
}

pub(super) fn probe_standalone(file_path: &str) -> Option<Vec<super::DurationDetail>> {
    let ext = super::extension_lc(file_path)?;
    if !matches!(ext.as_str(), "aac" | "adts") {
        return None;
    }
    super::standalone_detail(
        probe_seconds(file_path)?,
        DurationSourceKind::HeaderScan,
        true,
    )
}

fn probe_seconds(file_path: &str) -> Option<f64> {
    let mut file = File::open(file_path).ok()?;
    let info = scan_adts(&mut file)?;
    Some(info.frames as f64 * ADTS_SAMPLES_PER_FRAME as f64 / info.sample_rate as f64)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AdtsInfo {
    sample_rate: u32,
    frames: u64,
}

fn scan_adts<R: Read>(reader: &mut R) -> Option<AdtsInfo> {
    let mut data = Vec::new();
    reader.read_to_end(&mut data).ok()?;
    let mut pos = 0usize;
    let mut frames = 0u64;
    let mut sample_rate = 0u32;

    while pos + ADTS_HEADER_LEN <= data.len() {
        let header = &data[pos..pos + ADTS_HEADER_LEN];
        if header[0] != 0xff || (header[1] & 0xf0) != 0xf0 {
            return None;
        }
        let sr_index = ((header[2] & 0x3c) >> 2) as usize;
        let current_rate = *SAMPLE_RATES.get(sr_index)?;
        if current_rate == 0 {
            return None;
        }
        let frame_len = (((header[3] & 0x03) as usize) << 11)
            | ((header[4] as usize) << 3)
            | (((header[5] & 0xe0) as usize) >> 5);
        if frame_len < ADTS_HEADER_LEN || pos + frame_len > data.len() {
            return None;
        }
        if sample_rate == 0 {
            sample_rate = current_rate;
        } else if sample_rate != current_rate {
            return None;
        }
        frames += 1;
        pos += frame_len;
    }

    if pos == data.len() && frames > 0 && sample_rate > 0 {
        Some(AdtsInfo {
            sample_rate,
            frames,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adts_header(frame_len: usize) -> [u8; 7] {
        let mut h = [0u8; 7];
        h[0] = 0xff;
        h[1] = 0xf1;
        h[2] = (4 << 2) | 1;
        h[3] = ((frame_len >> 11) & 0x03) as u8;
        h[4] = ((frame_len >> 3) & 0xff) as u8;
        h[5] = (((frame_len & 0x07) << 5) as u8) | 0x1f;
        h[6] = 0xfc;
        h
    }

    #[test]
    fn scans_adts_headers_without_decoding() {
        let mut data = Vec::new();
        data.extend_from_slice(&adts_header(ADTS_HEADER_LEN));
        data.extend_from_slice(&adts_header(ADTS_HEADER_LEN));

        let info = scan_adts(&mut &data[..]).expect("adts");

        assert_eq!(info.sample_rate, 44_100);
        assert_eq!(info.frames, 2);
    }
}
