use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

use symphonia::core::formats::Track;

use super::{single_track_detail, DurationSourceKind};

pub(super) fn probe(file_path: &str, tracks: &[Track]) -> Option<Vec<super::DurationDetail>> {
    let ext = super::extension_lc(file_path)?;
    if !matches!(ext.as_str(), "mp4" | "m4a" | "aac") {
        return None;
    }

    let seconds = probe_seconds(file_path)?;
    single_track_detail(seconds, DurationSourceKind::Structural, true, tracks)
}

pub(super) fn probe_standalone(file_path: &str) -> Option<Vec<super::DurationDetail>> {
    let ext = super::extension_lc(file_path)?;
    if !matches!(ext.as_str(), "mp4" | "m4a" | "aac") {
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
    parse_mp4_duration(&mut file)
}

#[derive(Debug, Clone, Copy)]
struct Atom {
    name: [u8; 4],
    start: u64,
    header_len: u64,
    size: u64,
}

fn parse_mp4_duration<R: Read + Seek>(reader: &mut R) -> Option<f64> {
    let file_len = reader.seek(SeekFrom::End(0)).ok()?;
    reader.seek(SeekFrom::Start(0)).ok()?;
    let mut state = Mp4DurationState::default();
    parse_atoms_for_duration(reader, 0, file_len, &mut state);
    state.best_seconds()
}

fn parse_atoms_for_duration<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    end: u64,
    state: &mut Mp4DurationState,
) {
    let mut pos = start;
    while pos + 8 <= end {
        if reader.seek(SeekFrom::Start(pos)).is_err() {
            return;
        }
        let Some(atom) = read_atom(reader, pos, end) else {
            return;
        };
        let payload_start = atom.start + atom.header_len;
        let payload_end = atom.start + atom.size;
        if payload_end > end || atom.size < atom.header_len {
            return;
        }

        match &atom.name {
            b"mvhd" => {
                state.movie_timescale = parse_mvhd_timescale(reader, payload_start, payload_end);
            }
            b"mdhd" => {
                if let Some((timescale, duration)) = parse_mdhd(reader, payload_start, payload_end)
                {
                    state.media_timescale = Some(timescale);
                    state.mdhd_duration = Some(duration);
                }
            }
            b"elst" => {
                state.edit_duration = parse_elst_duration(reader, payload_start, payload_end);
            }
            b"stts" => {
                state.sample_table_duration =
                    parse_stts_duration(reader, payload_start, payload_end);
            }
            b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"edts" => {
                parse_atoms_for_duration(reader, payload_start, payload_end, state);
            }
            _ => {}
        }
        pos = payload_end;
    }
}

#[derive(Default)]
struct Mp4DurationState {
    movie_timescale: Option<u32>,
    media_timescale: Option<u32>,
    mdhd_duration: Option<u64>,
    edit_duration: Option<u64>,
    sample_table_duration: Option<u64>,
}

impl Mp4DurationState {
    fn best_seconds(&self) -> Option<f64> {
        if let (Some(duration), Some(timescale)) = (self.edit_duration, self.movie_timescale) {
            if duration > 0 && timescale > 0 {
                return Some(duration as f64 / timescale as f64);
            }
        }
        if let (Some(duration), Some(timescale)) = (self.mdhd_duration, self.media_timescale) {
            if duration > 0 && timescale > 0 {
                return Some(duration as f64 / timescale as f64);
            }
        }
        if let (Some(duration), Some(timescale)) =
            (self.sample_table_duration, self.media_timescale)
        {
            if duration > 0 && timescale > 0 {
                return Some(duration as f64 / timescale as f64);
            }
        }
        None
    }
}

fn read_atom<R: Read + Seek>(reader: &mut R, start: u64, parent_end: u64) -> Option<Atom> {
    let mut header = [0u8; 8];
    reader.read_exact(&mut header).ok()?;
    let size32 = u32::from_be_bytes(header[0..4].try_into().ok()?) as u64;
    let mut header_len = 8u64;
    let size = if size32 == 1 {
        let mut ext = [0u8; 8];
        reader.read_exact(&mut ext).ok()?;
        header_len = 16;
        u64::from_be_bytes(ext)
    } else if size32 == 0 {
        parent_end.checked_sub(start)?
    } else {
        size32
    };
    Some(Atom {
        name: header[4..8].try_into().ok()?,
        start,
        header_len,
        size,
    })
}

fn parse_mvhd_timescale<R: Read + Seek>(reader: &mut R, start: u64, end: u64) -> Option<u32> {
    reader.seek(SeekFrom::Start(start)).ok()?;
    let mut version_flags = [0u8; 4];
    reader.read_exact(&mut version_flags).ok()?;
    let version = version_flags[0];
    if version == 1 {
        if end < start + 32 {
            return None;
        }
        let mut data = [0u8; 20];
        reader.read_exact(&mut data).ok()?;
        nonzero_u32(u32::from_be_bytes(data[16..20].try_into().ok()?))
    } else {
        if end < start + 20 {
            return None;
        }
        let mut data = [0u8; 12];
        reader.read_exact(&mut data).ok()?;
        nonzero_u32(u32::from_be_bytes(data[8..12].try_into().ok()?))
    }
}

fn parse_mdhd<R: Read + Seek>(reader: &mut R, start: u64, end: u64) -> Option<(u32, u64)> {
    reader.seek(SeekFrom::Start(start)).ok()?;
    let mut version_flags = [0u8; 4];
    reader.read_exact(&mut version_flags).ok()?;
    let version = version_flags[0];
    if version == 1 {
        if end < start + 32 {
            return None;
        }
        let mut data = [0u8; 28];
        reader.read_exact(&mut data).ok()?;
        let timescale = nonzero_u32(u32::from_be_bytes(data[16..20].try_into().ok()?))?;
        let duration = u64::from_be_bytes(data[20..28].try_into().ok()?);
        Some((timescale, duration))
    } else {
        if end < start + 20 {
            return None;
        }
        let mut data = [0u8; 16];
        reader.read_exact(&mut data).ok()?;
        let timescale = nonzero_u32(u32::from_be_bytes(data[8..12].try_into().ok()?))?;
        let duration = u32::from_be_bytes(data[12..16].try_into().ok()?) as u64;
        Some((timescale, duration))
    }
}

fn parse_elst_duration<R: Read + Seek>(reader: &mut R, start: u64, end: u64) -> Option<u64> {
    reader.seek(SeekFrom::Start(start)).ok()?;
    let mut version_flags = [0u8; 4];
    reader.read_exact(&mut version_flags).ok()?;
    let version = version_flags[0];
    let mut count_bytes = [0u8; 4];
    reader.read_exact(&mut count_bytes).ok()?;
    let entry_count = u32::from_be_bytes(count_bytes) as u64;
    let entry_len = if version == 1 { 20 } else { 12 };
    if entry_count == 0 || end < start + 8 + entry_count.checked_mul(entry_len)? {
        return None;
    }

    let mut total = 0u64;
    for _ in 0..entry_count {
        if version == 1 {
            let mut data = [0u8; 20];
            reader.read_exact(&mut data).ok()?;
            total = total.checked_add(u64::from_be_bytes(data[0..8].try_into().ok()?))?;
        } else {
            let mut data = [0u8; 12];
            reader.read_exact(&mut data).ok()?;
            total = total.checked_add(u32::from_be_bytes(data[0..4].try_into().ok()?) as u64)?;
        }
    }
    nonzero_u64(total)
}

fn parse_stts_duration<R: Read + Seek>(reader: &mut R, start: u64, end: u64) -> Option<u64> {
    reader.seek(SeekFrom::Start(start)).ok()?;
    let mut header = [0u8; 8];
    reader.read_exact(&mut header).ok()?;
    let entry_count = u32::from_be_bytes(header[4..8].try_into().ok()?) as u64;
    if entry_count == 0 || end < start + 8 + entry_count.checked_mul(8)? {
        return None;
    }

    let mut total = 0u64;
    for _ in 0..entry_count {
        let mut entry = [0u8; 8];
        reader.read_exact(&mut entry).ok()?;
        let count = u32::from_be_bytes(entry[0..4].try_into().ok()?) as u64;
        let delta = u32::from_be_bytes(entry[4..8].try_into().ok()?) as u64;
        total = total.checked_add(count.checked_mul(delta)?)?;
    }
    nonzero_u64(total)
}

fn nonzero_u32(value: u32) -> Option<u32> {
    (value > 0).then_some(value)
}

fn nonzero_u64(value: u64) -> Option<u64> {
    (value > 0).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atom(name: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn parses_nested_mdhd_duration() {
        let mut mdhd = Vec::new();
        mdhd.extend_from_slice(&[0, 0, 0, 0]);
        mdhd.extend_from_slice(&0u32.to_be_bytes());
        mdhd.extend_from_slice(&0u32.to_be_bytes());
        mdhd.extend_from_slice(&44_100u32.to_be_bytes());
        mdhd.extend_from_slice(&88_200u32.to_be_bytes());
        let file = atom(
            b"moov",
            &atom(b"trak", &atom(b"mdia", &atom(b"mdhd", &mdhd))),
        );

        let seconds = parse_mp4_duration(&mut std::io::Cursor::new(file)).expect("duration");

        assert!((seconds - 2.0).abs() < 1e-9);
    }

    #[test]
    fn edit_list_duration_overrides_media_header_duration() {
        let mvhd = {
            let mut data = Vec::new();
            data.extend_from_slice(&[0, 0, 0, 0]);
            data.extend_from_slice(&0u32.to_be_bytes());
            data.extend_from_slice(&0u32.to_be_bytes());
            data.extend_from_slice(&1_000u32.to_be_bytes());
            data.extend_from_slice(&5_000u32.to_be_bytes());
            data
        };
        let mdhd = {
            let mut data = Vec::new();
            data.extend_from_slice(&[0, 0, 0, 0]);
            data.extend_from_slice(&0u32.to_be_bytes());
            data.extend_from_slice(&0u32.to_be_bytes());
            data.extend_from_slice(&44_100u32.to_be_bytes());
            data.extend_from_slice(&88_200u32.to_be_bytes());
            data
        };
        let elst = {
            let mut data = Vec::new();
            data.extend_from_slice(&[0, 0, 0, 0]);
            data.extend_from_slice(&1u32.to_be_bytes());
            data.extend_from_slice(&5_000u32.to_be_bytes());
            data.extend_from_slice(&0u32.to_be_bytes());
            data.extend_from_slice(&0x0001_0000u32.to_be_bytes());
            data
        };
        let file = atom(
            b"moov",
            &[
                atom(b"mvhd", &mvhd),
                atom(
                    b"trak",
                    &[
                        atom(b"edts", &atom(b"elst", &elst)),
                        atom(b"mdia", &atom(b"mdhd", &mdhd)),
                    ]
                    .concat(),
                ),
            ]
            .concat(),
        );

        let seconds = parse_mp4_duration(&mut std::io::Cursor::new(file)).expect("duration");

        assert!((seconds - 5.0).abs() < 1e-9);
    }

    #[test]
    fn sample_table_duration_fills_missing_media_header_duration() {
        let mdhd = {
            let mut data = Vec::new();
            data.extend_from_slice(&[0, 0, 0, 0]);
            data.extend_from_slice(&0u32.to_be_bytes());
            data.extend_from_slice(&0u32.to_be_bytes());
            data.extend_from_slice(&48_000u32.to_be_bytes());
            data.extend_from_slice(&0u32.to_be_bytes());
            data
        };
        let stts = {
            let mut data = Vec::new();
            data.extend_from_slice(&[0, 0, 0, 0]);
            data.extend_from_slice(&1u32.to_be_bytes());
            data.extend_from_slice(&100u32.to_be_bytes());
            data.extend_from_slice(&480u32.to_be_bytes());
            data
        };
        let file = atom(
            b"moov",
            &atom(
                b"trak",
                &atom(
                    b"mdia",
                    &[
                        atom(b"mdhd", &mdhd),
                        atom(b"minf", &atom(b"stbl", &atom(b"stts", &stts))),
                    ]
                    .concat(),
                ),
            ),
        );

        let seconds = parse_mp4_duration(&mut std::io::Cursor::new(file)).expect("duration");

        assert!((seconds - 1.0).abs() < 1e-9);
    }
}
