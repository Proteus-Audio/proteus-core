use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

use symphonia::core::formats::Track;

use super::{single_track_detail, DurationSourceKind};

const FORMAT_PCM: u16 = 0x0001;
const FORMAT_IEEE_FLOAT: u16 = 0x0003;
const FORMAT_EXTENSIBLE: u16 = 0xfffe;

pub(super) fn probe(file_path: &str, tracks: &[Track]) -> Option<Vec<super::DurationDetail>> {
    if super::extension_lc(file_path).as_deref() != Some("wav") {
        return None;
    }

    let seconds = probe_seconds(file_path)?;
    single_track_detail(seconds, DurationSourceKind::Structural, true, tracks)
}

pub(super) fn probe_standalone(file_path: &str) -> Option<Vec<super::DurationDetail>> {
    if super::extension_lc(file_path).as_deref() != Some("wav") {
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
    let info = parse_wav(&mut file)?;
    Some(info.data_bytes as f64 / info.block_align as f64 / info.sample_rate as f64)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WavInfo {
    sample_rate: u32,
    block_align: u16,
    data_bytes: u64,
}

fn parse_wav<R: Read + Seek>(reader: &mut R) -> Option<WavInfo> {
    let mut header = [0u8; 12];
    reader.read_exact(&mut header).ok()?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return None;
    }

    let mut fmt: Option<(u32, u16, u16)> = None;
    let mut data_bytes: Option<u64> = None;
    loop {
        let mut chunk = [0u8; 8];
        if reader.read_exact(&mut chunk).is_err() {
            break;
        }
        let chunk_id = &chunk[0..4];
        let chunk_size = u32::from_le_bytes(chunk[4..8].try_into().ok()?) as u64;

        if chunk_id == b"fmt " {
            let mut data = vec![0u8; chunk_size as usize];
            reader.read_exact(&mut data).ok()?;
            if data.len() < 16 {
                return None;
            }
            let audio_format = u16::from_le_bytes(data[0..2].try_into().ok()?);
            let sample_rate = u32::from_le_bytes(data[4..8].try_into().ok()?);
            let block_align = u16::from_le_bytes(data[12..14].try_into().ok()?);
            let format_supported = match audio_format {
                FORMAT_PCM | FORMAT_IEEE_FLOAT => true,
                FORMAT_EXTENSIBLE => extensible_subformat_supported(&data),
                _ => false,
            };
            if format_supported && sample_rate > 0 && block_align > 0 {
                fmt = Some((sample_rate, block_align, audio_format));
            }
        } else {
            if chunk_id == b"data" {
                data_bytes = Some(chunk_size);
            }
            let skip = chunk_size + (chunk_size % 2);
            reader.seek(SeekFrom::Current(skip as i64)).ok()?;
        }

        if fmt.is_some() && data_bytes.is_some() {
            break;
        }
    }

    let (sample_rate, block_align, _) = fmt?;
    Some(WavInfo {
        sample_rate,
        block_align,
        data_bytes: data_bytes?,
    })
}

fn extensible_subformat_supported(data: &[u8]) -> bool {
    if data.len() < 40 {
        return false;
    }
    let subformat_tag = u16::from_le_bytes([data[24], data[25]]);
    subformat_tag == FORMAT_PCM || subformat_tag == FORMAT_IEEE_FLOAT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pcm_wav_duration_fields() {
        let mut data = Vec::new();
        data.extend_from_slice(b"RIFF");
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(b"WAVE");
        data.extend_from_slice(b"fmt ");
        data.extend_from_slice(&16u32.to_le_bytes());
        data.extend_from_slice(&FORMAT_PCM.to_le_bytes());
        data.extend_from_slice(&2u16.to_le_bytes());
        data.extend_from_slice(&44_100u32.to_le_bytes());
        data.extend_from_slice(&176_400u32.to_le_bytes());
        data.extend_from_slice(&4u16.to_le_bytes());
        data.extend_from_slice(&16u16.to_le_bytes());
        data.extend_from_slice(b"data");
        data.extend_from_slice(&176_400u32.to_le_bytes());

        let info = parse_wav(&mut std::io::Cursor::new(data)).expect("wav");

        assert_eq!(info.sample_rate, 44_100);
        assert_eq!(info.block_align, 4);
        assert_eq!(info.data_bytes, 176_400);
    }
}
