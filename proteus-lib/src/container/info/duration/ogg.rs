//! Ogg page duration probing.

use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use symphonia::core::formats::Track;

const OGG_CAPTURE: &[u8; 4] = b"OggS";
const OGG_HEADER_LEN: usize = 27;
const INITIAL_TAIL_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy)]
struct OggPage {
    granule_position: i64,
    stream_serial: u32,
    total_len: usize,
}

pub(super) fn probe_durations(file_path: &str, tracks: &[Track]) -> Option<HashMap<u32, f64>> {
    if !is_ogg_path(file_path) || tracks.is_empty() {
        return None;
    }

    let expected: HashMap<u32, f64> = tracks
        .iter()
        .filter_map(|track| {
            let sample_rate = track.codec_params.sample_rate?;
            if sample_rate == 0 {
                return None;
            }
            Some((track.id, sample_rate as f64))
        })
        .collect();
    if expected.is_empty() {
        return None;
    }

    let mut file = File::open(file_path).ok()?;
    let file_len = file.metadata().ok()?.len();
    if file_len < OGG_HEADER_LEN as u64 {
        return None;
    }

    let mut tail_len = INITIAL_TAIL_BYTES.min(file_len);
    loop {
        let durations = probe_tail(&mut file, file_len, tail_len, &expected)?;
        if durations.len() == expected.len() || tail_len == file_len {
            return if durations.is_empty() {
                None
            } else {
                Some(durations)
            };
        }

        tail_len = (tail_len * 2).min(file_len);
    }
}

fn is_ogg_path(file_path: &str) -> bool {
    matches!(
        Path::new(file_path)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase()),
        Some(extension) if extension == "ogg" || extension == "opus"
    )
}

fn probe_tail(
    file: &mut File,
    file_len: u64,
    tail_len: u64,
    expected: &HashMap<u32, f64>,
) -> Option<HashMap<u32, f64>> {
    let start = file_len.checked_sub(tail_len)?;
    file.seek(SeekFrom::Start(start)).ok()?;

    let mut data = vec![0u8; tail_len as usize];
    file.read_exact(&mut data).ok()?;

    let mut last_granule: HashMap<u32, u64> = HashMap::new();
    let mut pos = 0usize;
    while let Some(rel) = find_capture(&data[pos..]) {
        let page_offset = pos + rel;
        let Some(page) = parse_page(&data[page_offset..]) else {
            pos = page_offset + OGG_CAPTURE.len();
            continue;
        };

        if page.granule_position >= 0 && expected.contains_key(&page.stream_serial) {
            last_granule.insert(page.stream_serial, page.granule_position as u64);
        }
        pos = page_offset + page.total_len.max(OGG_CAPTURE.len());
    }

    let durations = last_granule
        .into_iter()
        .filter_map(|(track_id, granule_position)| {
            let sample_rate = expected.get(&track_id)?;
            Some((track_id, granule_position as f64 / sample_rate))
        })
        .collect::<HashMap<_, _>>();

    Some(durations)
}

fn find_capture(data: &[u8]) -> Option<usize> {
    data.windows(OGG_CAPTURE.len())
        .position(|window| window == OGG_CAPTURE)
}

fn parse_page(data: &[u8]) -> Option<OggPage> {
    if data.len() < OGG_HEADER_LEN || &data[..OGG_CAPTURE.len()] != OGG_CAPTURE {
        return None;
    }
    if data[4] != 0 {
        return None;
    }

    let page_segments = data[26] as usize;
    let segment_table_end = OGG_HEADER_LEN.checked_add(page_segments)?;
    if data.len() < segment_table_end {
        return None;
    }

    let body_len = data[OGG_HEADER_LEN..segment_table_end]
        .iter()
        .try_fold(0usize, |acc, segment_len| {
            acc.checked_add(*segment_len as usize)
        })?;
    let total_len = segment_table_end.checked_add(body_len)?;
    if data.len() < total_len {
        return None;
    }

    let granule_position = i64::from_le_bytes(data[6..14].try_into().ok()?);
    let stream_serial = u32::from_le_bytes(data[14..18].try_into().ok()?);

    Some(OggPage {
        granule_position,
        stream_serial,
        total_len,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_page_reads_granule_and_serial() {
        let mut page = vec![0u8; OGG_HEADER_LEN];
        page[..4].copy_from_slice(OGG_CAPTURE);
        page[4] = 0;
        page[6..14].copy_from_slice(&48_000i64.to_le_bytes());
        page[14..18].copy_from_slice(&1234u32.to_le_bytes());
        page[26] = 0;

        let parsed = parse_page(&page).expect("page");

        assert_eq!(parsed.granule_position, 48_000);
        assert_eq!(parsed.stream_serial, 1234);
        assert_eq!(parsed.total_len, OGG_HEADER_LEN);
    }

    #[test]
    fn parse_page_rejects_incomplete_page() {
        let mut page = vec![0u8; OGG_HEADER_LEN];
        page[..4].copy_from_slice(OGG_CAPTURE);
        page[4] = 0;
        page[26] = 1;
        page.push(255);

        assert!(parse_page(&page).is_none());
    }
}
