use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

use symphonia::core::formats::Track;

use super::{single_track_detail, DurationSourceKind};

const DEFAULT_TIMESTAMP_SCALE_NS: u64 = 1_000_000;
const TAIL_SCAN_BYTES: u64 = 1024 * 1024;

pub(super) fn probe(file_path: &str, tracks: &[Track]) -> Option<Vec<super::DurationDetail>> {
    let ext = super::extension_lc(file_path)?;
    if !matches!(ext.as_str(), "mka" | "mkv" | "webm" | "prot") {
        return None;
    }

    let file = File::open(file_path).ok()?;
    let matroska = matroska::Matroska::open(file).ok()?;
    let seconds = matroska
        .info
        .duration
        .map(|duration| duration.as_secs_f64())
        .or_else(|| probe_last_cluster_seconds(file_path))?;
    single_track_detail(seconds, DurationSourceKind::Structural, true, tracks)
}

fn probe_last_cluster_seconds(file_path: &str) -> Option<f64> {
    let mut file = File::open(file_path).ok()?;
    let file_len = file.seek(SeekFrom::End(0)).ok()?;
    let tail_len = file_len.min(TAIL_SCAN_BYTES);
    file.seek(SeekFrom::Start(file_len - tail_len)).ok()?;
    let mut data = vec![0u8; tail_len as usize];
    file.read_exact(&mut data).ok()?;
    last_cluster_seconds(&data, DEFAULT_TIMESTAMP_SCALE_NS)
}

fn last_cluster_seconds(data: &[u8], timestamp_scale_ns: u64) -> Option<f64> {
    let cluster_pos = find_last(data, &[0x1f, 0x43, 0xb6, 0x75])?;
    let cluster = &data[cluster_pos + 4..];
    let (_, size_len, cluster_size) = read_ebml_vint(cluster, false)?;
    let cluster_start = size_len;
    let cluster_end = cluster_size
        .and_then(|size| cluster_start.checked_add(size as usize))
        .filter(|end| *end <= cluster.len())
        .unwrap_or(cluster.len());
    let cluster = &cluster[cluster_start..cluster_end];
    let cluster_timecode = read_unsigned_child(cluster, 0xe7)?;
    let block_timecode = max_block_timecode(cluster).unwrap_or(0);
    let timestamp = (cluster_timecode as i64).checked_add(block_timecode as i64)?;
    if timestamp <= 0 || timestamp_scale_ns == 0 {
        return None;
    }
    Some(timestamp as f64 * timestamp_scale_ns as f64 / 1_000_000_000.0)
}

fn read_unsigned_child(data: &[u8], target_id: u64) -> Option<u64> {
    let mut pos = 0usize;
    while pos < data.len() {
        let (id, id_len, _) = read_ebml_vint(&data[pos..], true)?;
        let after_id = pos.checked_add(id_len)?;
        let (_, size_len, size) = read_ebml_vint(&data[after_id..], false)?;
        let value_start = after_id.checked_add(size_len)?;
        let value_len = size? as usize;
        let value_end = value_start.checked_add(value_len)?;
        if value_end > data.len() {
            return None;
        }
        if id == target_id {
            let mut value = 0u64;
            for byte in &data[value_start..value_end] {
                value = (value << 8) | *byte as u64;
            }
            return Some(value);
        }
        pos = value_end;
    }
    None
}

fn max_block_timecode(data: &[u8]) -> Option<i16> {
    let mut pos = 0usize;
    let mut max_timecode = None;
    while pos < data.len() {
        let (id, id_len, _) = read_ebml_vint(&data[pos..], true)?;
        let after_id = pos.checked_add(id_len)?;
        let (_, size_len, size) = read_ebml_vint(&data[after_id..], false)?;
        let value_start = after_id.checked_add(size_len)?;
        let value_end = value_start.checked_add(size? as usize)?;
        if value_end > data.len() {
            return max_timecode;
        }

        match id {
            0xa3 | 0xa1 => {
                if let Some(timecode) = parse_block_timecode(&data[value_start..value_end]) {
                    max_timecode =
                        Some(max_timecode.map_or(timecode, |current: i16| current.max(timecode)));
                }
            }
            0xa0 => {
                if let Some(timecode) = max_block_timecode(&data[value_start..value_end]) {
                    max_timecode =
                        Some(max_timecode.map_or(timecode, |current: i16| current.max(timecode)));
                }
            }
            _ => {}
        }
        pos = value_end;
    }
    max_timecode
}

fn parse_block_timecode(data: &[u8]) -> Option<i16> {
    let (_, track_len, _) = read_ebml_vint(data, false)?;
    let timecode_start = track_len;
    let timecode_end = timecode_start.checked_add(2)?;
    let bytes = data.get(timecode_start..timecode_end)?;
    Some(i16::from_be_bytes(bytes.try_into().ok()?))
}

fn read_ebml_vint(data: &[u8], keep_marker: bool) -> Option<(u64, usize, Option<u64>)> {
    let first = *data.first()?;
    let leading = first.leading_zeros() as usize;
    if leading >= 8 {
        return None;
    }
    let len = leading + 1;
    if data.len() < len {
        return None;
    }

    let marker = 1u8 << (8 - len);
    let mut value = if keep_marker {
        first as u64
    } else {
        (first & !marker) as u64
    };
    for byte in &data[1..len] {
        value = (value << 8) | *byte as u64;
    }

    let unknown = !keep_marker && value == ((1u64 << (7 * len)) - 1);
    Some((value, len, (!unknown).then_some(value)))
}

fn find_last(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vint_size(value: u8) -> u8 {
        0x80 | value
    }

    fn element(id: &[u8], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(id);
        out.push(vint_size(payload.len() as u8));
        out.extend_from_slice(payload);
        out
    }

    fn uint_element(id: &[u8], value: u64) -> Vec<u8> {
        let bytes = if value <= u8::MAX as u64 {
            vec![value as u8]
        } else {
            (value as u16).to_be_bytes().to_vec()
        };
        element(id, &bytes)
    }

    fn simple_block(timecode: i16) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.push(0x81);
        payload.extend_from_slice(&timecode.to_be_bytes());
        payload.push(0x80);
        element(&[0xa3], &payload)
    }

    #[test]
    fn last_cluster_timestamp_uses_cluster_and_block_timecodes() {
        let mut cluster = Vec::new();
        cluster.extend_from_slice(&uint_element(&[0xe7], 41_000));
        cluster.extend_from_slice(&simple_block(668));
        let file_tail = element(&[0x1f, 0x43, 0xb6, 0x75], &cluster);

        let seconds = last_cluster_seconds(&file_tail, DEFAULT_TIMESTAMP_SCALE_NS).expect("time");

        assert!((seconds - 41.668).abs() < 1e-9);
    }
}
