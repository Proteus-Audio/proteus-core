use std::{fs::File, io::Read};

use symphonia::core::formats::Track;

use super::{single_track_detail, DurationSourceKind};

const BITRATES_MPEG1: [u16; 16] = [
    0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
];
const BITRATES_MPEG2: [u16; 16] = [
    0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
];
const SAMPLE_RATES: [[u32; 4]; 4] = [
    [0, 0, 0, 0],
    [22_050, 24_000, 16_000, 0],
    [44_100, 48_000, 32_000, 0],
    [44_100, 48_000, 32_000, 0],
];

pub(super) fn probe(file_path: &str, tracks: &[Track]) -> Option<Vec<super::DurationDetail>> {
    if super::extension_lc(file_path).as_deref() != Some("mp3") {
        return None;
    }

    let info = probe_info(file_path)?;
    let seconds = info.frames as f64 * info.samples_per_frame as f64 / info.sample_rate as f64;
    single_track_detail(seconds, info.source, info.exact, tracks)
}

pub(super) fn probe_standalone(file_path: &str) -> Option<Vec<super::DurationDetail>> {
    if super::extension_lc(file_path).as_deref() != Some("mp3") {
        return None;
    }
    let info = probe_info(file_path)?;
    let seconds = info.frames as f64 * info.samples_per_frame as f64 / info.sample_rate as f64;
    super::standalone_detail(seconds, info.source, info.exact)
}

fn probe_info(file_path: &str) -> Option<Mp3Info> {
    let mut file = File::open(file_path).ok()?;
    parse_mp3(&mut file)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Mp3Info {
    sample_rate: u32,
    samples_per_frame: u16,
    frames: u64,
    source: DurationSourceKind,
    exact: bool,
}

#[derive(Debug, Clone, Copy)]
struct Mp3Frame {
    sample_rate: u32,
    frame_len: usize,
    samples_per_frame: u16,
    channels: u8,
    version: u8,
}

fn parse_mp3<R: Read>(reader: &mut R) -> Option<Mp3Info> {
    let mut data = Vec::new();
    reader.read_to_end(&mut data).ok()?;
    let first = find_first_frame(&data)?;
    let frame = parse_frame_header(&data[first..first + 4])?;

    if let Some(frames) = parse_xing_frames(&data[first..], frame) {
        return Some(Mp3Info {
            sample_rate: frame.sample_rate,
            samples_per_frame: frame.samples_per_frame,
            frames,
            source: DurationSourceKind::Structural,
            exact: true,
        });
    }
    if let Some(frames) = parse_vbri_frames(&data[first..]) {
        return Some(Mp3Info {
            sample_rate: frame.sample_rate,
            samples_per_frame: frame.samples_per_frame,
            frames,
            source: DurationSourceKind::Structural,
            exact: true,
        });
    }

    scan_frames(&data[first..])
}

fn find_first_frame(data: &[u8]) -> Option<usize> {
    let start = if data.len() >= 10 && &data[0..3] == b"ID3" {
        let size = synchsafe_to_u32(&data[6..10])? as usize;
        10usize.checked_add(size)?
    } else {
        0
    };
    (start..data.len().saturating_sub(4))
        .find(|pos| parse_frame_header(&data[*pos..*pos + 4]).is_some())
}

fn synchsafe_to_u32(bytes: &[u8]) -> Option<u32> {
    if bytes.len() != 4 || bytes.iter().any(|byte| (byte & 0x80) != 0) {
        return None;
    }
    Some(
        ((bytes[0] as u32) << 21)
            | ((bytes[1] as u32) << 14)
            | ((bytes[2] as u32) << 7)
            | bytes[3] as u32,
    )
}

fn parse_frame_header(header: &[u8]) -> Option<Mp3Frame> {
    if header.len() < 4 || header[0] != 0xff || (header[1] & 0xe0) != 0xe0 {
        return None;
    }
    let version = (header[1] >> 3) & 0x03;
    let layer = (header[1] >> 1) & 0x03;
    if version == 0 || layer != 1 {
        return None;
    }
    let bitrate_index = (header[2] >> 4) as usize;
    let sample_rate_index = ((header[2] >> 2) & 0x03) as usize;
    let padding = ((header[2] >> 1) & 0x01) as usize;
    let channel_mode = (header[3] >> 6) & 0x03;

    let sample_rate = SAMPLE_RATES[version as usize][sample_rate_index];
    let bitrate = if version == 3 {
        BITRATES_MPEG1[bitrate_index]
    } else {
        BITRATES_MPEG2[bitrate_index]
    } as usize;
    if sample_rate == 0 || bitrate == 0 {
        return None;
    }

    let samples_per_frame = if version == 3 { 1152 } else { 576 };
    let coeff = if version == 3 { 144_000 } else { 72_000 };
    let frame_len = (coeff * bitrate / sample_rate as usize) + padding;
    if frame_len < 4 {
        return None;
    }

    Some(Mp3Frame {
        sample_rate,
        frame_len,
        samples_per_frame,
        channels: if channel_mode == 3 { 1 } else { 2 },
        version,
    })
}

fn parse_xing_frames(data: &[u8], frame: Mp3Frame) -> Option<u64> {
    let side_info = match (frame.version == 3, frame.channels == 1) {
        (true, true) => 17,
        (true, false) => 32,
        (false, true) => 9,
        (false, false) => 17,
    };
    let offset = 4usize.checked_add(side_info)?;
    if data.len() < offset + 12 {
        return None;
    }
    let tag = &data[offset..offset + 4];
    if tag != b"Xing" && tag != b"Info" {
        return None;
    }
    let flags = u32::from_be_bytes(data[offset + 4..offset + 8].try_into().ok()?);
    if (flags & 0x01) == 0 {
        return None;
    }
    Some(u32::from_be_bytes(data[offset + 8..offset + 12].try_into().ok()?) as u64)
}

fn parse_vbri_frames(data: &[u8]) -> Option<u64> {
    if data.len() < 36 || &data[32..36] != b"VBRI" || data.len() < 48 {
        return None;
    }
    Some(u32::from_be_bytes(data[44..48].try_into().ok()?) as u64)
}

fn scan_frames(data: &[u8]) -> Option<Mp3Info> {
    let mut pos = 0usize;
    let mut frames = 0u64;
    let mut first: Option<Mp3Frame> = None;
    while pos + 4 <= data.len() {
        let frame = parse_frame_header(&data[pos..pos + 4])?;
        if pos + frame.frame_len > data.len() {
            break;
        }
        first.get_or_insert(frame);
        frames += 1;
        pos += frame.frame_len;
    }

    let frame = first?;
    if frames == 0 {
        return None;
    }
    Some(Mp3Info {
        sample_rate: frame.sample_rate,
        samples_per_frame: frame.samples_per_frame,
        frames,
        source: DurationSourceKind::HeaderScan,
        exact: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mpeg1_layer3_header() {
        let frame = parse_frame_header(&[0xff, 0xfb, 0x90, 0x64]).expect("frame");

        assert_eq!(frame.sample_rate, 44_100);
        assert_eq!(frame.samples_per_frame, 1152);
        assert_eq!(frame.channels, 2);
    }

    #[test]
    fn parses_xing_frame_count() {
        let frame = parse_frame_header(&[0xff, 0xfb, 0x90, 0x64]).expect("frame");
        let mut data = vec![0u8; 4 + 32 + 12];
        data[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0x64]);
        data[36..40].copy_from_slice(b"Xing");
        data[40..44].copy_from_slice(&1u32.to_be_bytes());
        data[44..48].copy_from_slice(&345u32.to_be_bytes());

        assert_eq!(parse_xing_frames(&data, frame), Some(345));
    }
}
