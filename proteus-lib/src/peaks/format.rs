use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};

use super::{PeakWindow, PeaksData, PeaksError};

const MAGIC: [u8; 8] = *b"PPEAKS01";
const VERSION: u16 = 1;
const HEADER_SIZE: u64 = 64;
const HEADER_BYTES_USED: usize = 36;
const PEAK_BYTES_PER_CHANNEL: u64 = 8; // max f32 + min f32

struct Header {
    channels: u16,
    sample_rate: u32,
    window_size: u32,
    peak_count: u64,
    data_offset: u64,
}

pub(crate) fn write_peaks_file(path: &str, peaks: &PeaksData) -> Result<(), PeaksError> {
    if peaks.channels.is_empty() {
        return Err(PeaksError::InvalidFormat(
            "peaks must contain at least one channel".to_string(),
        ));
    }

    if peaks.window_size == 0 {
        return Err(PeaksError::InvalidFormat(
            "window_size must be greater than zero".to_string(),
        ));
    }

    let channels_u16 = u16::try_from(peaks.channels.len()).map_err(|_| {
        PeaksError::InvalidFormat("number of channels exceeds u16 range".to_string())
    })?;

    let peak_count = peaks.channels[0].len();
    for channel in &peaks.channels {
        if channel.len() != peak_count {
            return Err(PeaksError::InvalidFormat(
                "all channels must have the same peak length".to_string(),
            ));
        }
    }

    let mut writer = BufWriter::new(File::create(path)?);
    let header = Header {
        channels: channels_u16,
        sample_rate: peaks.sample_rate,
        window_size: peaks.window_size,
        peak_count: peak_count as u64,
        data_offset: HEADER_SIZE,
    };

    write_header(&mut writer, &header)?;
    for i in 0..peak_count {
        for channel in &peaks.channels {
            writer.write_all(&channel[i].max.to_le_bytes())?;
            writer.write_all(&channel[i].min.to_le_bytes())?;
        }
    }
    writer.flush()?;
    Ok(())
}

pub(crate) fn read_peaks_file(path: &str) -> Result<PeaksData, PeaksError> {
    let mut reader = BufReader::new(File::open(path)?);
    let header = read_header(&mut reader)?;
    read_peaks_by_indices(&mut reader, &header, 0, header.peak_count)
}

pub(crate) fn read_peaks_in_range(
    path: &str,
    start_seconds: f64,
    end_seconds: f64,
) -> Result<PeaksData, PeaksError> {
    if !start_seconds.is_finite() || !end_seconds.is_finite() {
        return Err(PeaksError::InvalidFormat(
            "timestamps must be finite numbers".to_string(),
        ));
    }

    if start_seconds < 0.0 || end_seconds < 0.0 {
        return Err(PeaksError::InvalidFormat(
            "timestamps must be >= 0.0".to_string(),
        ));
    }

    if end_seconds < start_seconds {
        return Err(PeaksError::InvalidFormat(
            "end_seconds must be >= start_seconds".to_string(),
        ));
    }

    let mut reader = BufReader::new(File::open(path)?);
    let header = read_header(&mut reader)?;
    let samples_per_peak = u64::from(header.window_size);
    let sample_rate = f64::from(header.sample_rate);

    let start_sample = (start_seconds * sample_rate).floor() as u64;
    let end_sample = (end_seconds * sample_rate).ceil() as u64;

    let start_peak = start_sample / samples_per_peak;
    let mut end_peak = end_sample.div_ceil(samples_per_peak);

    let peak_count = header.peak_count;
    let clamped_start = start_peak.min(peak_count);
    end_peak = end_peak.min(peak_count);
    let clamped_end = end_peak.max(clamped_start);

    read_peaks_by_indices(&mut reader, &header, clamped_start, clamped_end)
}

fn read_peaks_by_indices<R: Read + Seek>(
    reader: &mut R,
    header: &Header,
    start_peak: u64,
    end_peak: u64,
) -> Result<PeaksData, PeaksError> {
    if end_peak < start_peak {
        return Err(PeaksError::InvalidFormat(
            "invalid peak index range".to_string(),
        ));
    }

    let channels = usize::from(header.channels);
    let sample_count = end_peak - start_peak;
    let samples_len = usize::try_from(sample_count).map_err(|_| {
        PeaksError::InvalidFormat("peak range exceeds addressable memory size".to_string())
    })?;

    let bytes_per_peak = u64::from(header.channels) * PEAK_BYTES_PER_CHANNEL;
    let start_offset = header
        .data_offset
        .checked_add(start_peak.saturating_mul(bytes_per_peak))
        .ok_or_else(|| PeaksError::InvalidFormat("computed start offset overflow".to_string()))?;
    reader.seek(SeekFrom::Start(start_offset))?;

    let mut channel_data = vec![Vec::with_capacity(samples_len); channels];
    let mut f32_buf = [0_u8; 4];

    for _ in start_peak..end_peak {
        for channel in &mut channel_data {
            reader.read_exact(&mut f32_buf)?;
            let max = f32::from_le_bytes(f32_buf);
            reader.read_exact(&mut f32_buf)?;
            let min = f32::from_le_bytes(f32_buf);
            channel.push(PeakWindow { max, min });
        }
    }

    Ok(PeaksData {
        sample_rate: header.sample_rate,
        window_size: header.window_size,
        channels: channel_data,
    })
}

fn write_header<W: Write>(writer: &mut W, header: &Header) -> Result<(), PeaksError> {
    writer.write_all(&MAGIC)?;
    writer.write_all(&VERSION.to_le_bytes())?;
    writer.write_all(&header.channels.to_le_bytes())?;
    writer.write_all(&header.sample_rate.to_le_bytes())?;
    writer.write_all(&header.window_size.to_le_bytes())?;
    writer.write_all(&header.peak_count.to_le_bytes())?;
    writer.write_all(&header.data_offset.to_le_bytes())?;

    let padding_len = HEADER_SIZE as usize - HEADER_BYTES_USED;
    writer.write_all(&vec![0_u8; padding_len])?;
    Ok(())
}

fn read_header<R: Read>(reader: &mut R) -> Result<Header, PeaksError> {
    let mut header = [0_u8; HEADER_SIZE as usize];
    reader.read_exact(&mut header)?;

    if header[0..8] != MAGIC {
        return Err(PeaksError::InvalidFormat(
            "magic mismatch: expected PPEAKS01".to_string(),
        ));
    }

    let version = u16::from_le_bytes([header[8], header[9]]);
    if version != VERSION {
        return Err(PeaksError::InvalidFormat(format!(
            "unsupported peaks version: {}",
            version
        )));
    }

    let channels = u16::from_le_bytes([header[10], header[11]]);
    if channels == 0 {
        return Err(PeaksError::InvalidFormat(
            "channel count cannot be zero".to_string(),
        ));
    }

    let sample_rate = u32::from_le_bytes([header[12], header[13], header[14], header[15]]);
    if sample_rate == 0 {
        return Err(PeaksError::InvalidFormat(
            "sample_rate cannot be zero".to_string(),
        ));
    }

    let window_size = u32::from_le_bytes([header[16], header[17], header[18], header[19]]);
    if window_size == 0 {
        return Err(PeaksError::InvalidFormat(
            "window_size cannot be zero".to_string(),
        ));
    }

    let peak_count = u64::from_le_bytes([
        header[20], header[21], header[22], header[23], header[24], header[25], header[26],
        header[27],
    ]);
    let data_offset = u64::from_le_bytes([
        header[28], header[29], header[30], header[31], header[32], header[33], header[34],
        header[35],
    ]);

    if data_offset < HEADER_SIZE {
        return Err(PeaksError::InvalidFormat(
            "data_offset is smaller than header size".to_string(),
        ));
    }

    Ok(Header {
        channels,
        sample_rate,
        window_size,
        peak_count,
        data_offset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_file_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("proteus-peaks-{}.bin", nanos))
    }

    #[test]
    fn round_trips_full_peaks_file() {
        let path = test_file_path();
        let data = PeaksData {
            sample_rate: 48_000,
            window_size: 480,
            channels: vec![
                vec![
                    PeakWindow {
                        max: 0.5,
                        min: -0.5,
                    },
                    PeakWindow {
                        max: 0.2,
                        min: -0.1,
                    },
                ],
                vec![
                    PeakWindow {
                        max: 0.4,
                        min: -0.4,
                    },
                    PeakWindow {
                        max: 0.1,
                        min: -0.2,
                    },
                ],
            ],
        };

        write_peaks_file(path.to_str().unwrap(), &data).expect("write");
        let read_back = read_peaks_file(path.to_str().unwrap()).expect("read");

        assert_eq!(read_back.sample_rate, 48_000);
        assert_eq!(read_back.window_size, 480);
        assert_eq!(read_back.channels.len(), 2);
        assert_eq!(read_back.channels[0].len(), 2);
        assert_eq!(read_back.channels[0][0].max, 0.5);
        assert_eq!(read_back.channels[1][1].min, -0.2);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn reads_peak_range() {
        let path = test_file_path();
        let data = PeaksData {
            sample_rate: 10,
            window_size: 2,
            channels: vec![vec![
                PeakWindow {
                    max: 1.0,
                    min: -1.0,
                },
                PeakWindow {
                    max: 2.0,
                    min: -2.0,
                },
                PeakWindow {
                    max: 3.0,
                    min: -3.0,
                },
            ]],
        };

        write_peaks_file(path.to_str().unwrap(), &data).expect("write");
        let slice = read_peaks_in_range(path.to_str().unwrap(), 0.2, 0.6).expect("range");

        assert_eq!(slice.channels.len(), 1);
        assert_eq!(slice.channels[0].len(), 2);
        assert_eq!(slice.channels[0][0].max, 2.0);
        assert_eq!(slice.channels[0][1].max, 3.0);

        let _ = std::fs::remove_file(path);
    }
}
