use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};

use super::{GetPeaksOptions, PeakWindow, PeaksData, PeaksError};

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

pub(crate) fn read_peaks_with_options(
    path: &str,
    options: &GetPeaksOptions,
) -> Result<PeaksData, PeaksError> {
    if options.target_peaks == Some(0) {
        return Err(PeaksError::InvalidFormat(
            "target_peaks must be greater than zero".to_string(),
        ));
    }

    if options.channels == Some(0) {
        return Err(PeaksError::InvalidFormat(
            "channels must be greater than zero".to_string(),
        ));
    }

    let mut reader = BufReader::new(File::open(path)?);
    let header = read_header(&mut reader)?;
    let (requested_start_sample, requested_end_sample) =
        compute_requested_sample_range(&header, options.start_seconds, options.end_seconds)?;
    let (start_peak, end_peak) =
        compute_peak_range(&header, requested_start_sample, requested_end_sample);
    let mut peaks = read_peaks_by_indices(&mut reader, &header, start_peak, end_peak)?;

    if let Some(requested_channels) = options.channels {
        peaks
            .channels
            .truncate(requested_channels.min(peaks.channels.len()));
    }

    if let Some(target_peaks) = options.target_peaks {
        if options.start_seconds.is_some() && options.end_seconds.is_some() {
            peaks = time_align_peaks(
                &peaks,
                &header,
                start_peak,
                requested_start_sample,
                requested_end_sample,
                target_peaks,
            );
        } else {
            downsample_peaks(&mut peaks, target_peaks);
        }
    }

    Ok(peaks)
}

fn compute_requested_sample_range(
    header: &Header,
    start_seconds: Option<f64>,
    end_seconds: Option<f64>,
) -> Result<(u64, u64), PeaksError> {
    let mut start = start_seconds.unwrap_or(0.0);
    let mut end = end_seconds.unwrap_or(f64::MAX);

    if !start.is_finite() || !end.is_finite() {
        return Err(PeaksError::InvalidFormat(
            "timestamps must be finite numbers".to_string(),
        ));
    }

    if start < 0.0 || end < 0.0 {
        return Err(PeaksError::InvalidFormat(
            "timestamps must be >= 0.0".to_string(),
        ));
    }

    if end < start {
        return Err(PeaksError::InvalidFormat(
            "end_seconds must be >= start_seconds".to_string(),
        ));
    }

    let sample_rate = f64::from(header.sample_rate);

    if end == f64::MAX {
        end = total_samples(header) as f64 / sample_rate;
    }

    // Keep values stable for very large ranges.
    start = start.min(u64::MAX as f64 / sample_rate);
    end = end.min(u64::MAX as f64 / sample_rate);

    let start_sample = (start * sample_rate).floor() as u64;
    let end_sample = (end * sample_rate).ceil() as u64;

    Ok((start_sample, end_sample))
}

fn compute_peak_range(header: &Header, start_sample: u64, end_sample: u64) -> (u64, u64) {
    let samples_per_peak = u64::from(header.window_size);
    let start_peak = start_sample / samples_per_peak;
    let mut end_peak = end_sample.div_ceil(samples_per_peak);

    let peak_count = header.peak_count;
    let clamped_start = start_peak.min(peak_count);
    end_peak = end_peak.min(peak_count);
    let clamped_end = end_peak.max(clamped_start);

    (clamped_start, clamped_end)
}

fn time_align_peaks(
    peaks: &PeaksData,
    header: &Header,
    start_peak: u64,
    requested_start_sample: u64,
    requested_end_sample: u64,
    target_peaks: usize,
) -> PeaksData {
    if target_peaks == 0 {
        return PeaksData {
            sample_rate: peaks.sample_rate,
            window_size: peaks.window_size,
            channels: peaks
                .channels
                .iter()
                .map(|_| Vec::new())
                .collect::<Vec<Vec<PeakWindow>>>(),
        };
    }

    let duration_samples = requested_end_sample.saturating_sub(requested_start_sample) as f64;
    let samples_per_peak = f64::from(header.window_size);
    let available_peak_count = peaks.channels.first().map_or(0, |channel| channel.len());
    let end_peak = start_peak.saturating_add(available_peak_count as u64);
    let total_samples = total_samples(header) as f64;

    let channels = peaks
        .channels
        .iter()
        .map(|channel| {
            let mut aligned = Vec::with_capacity(target_peaks);
            for i in 0..target_peaks {
                let bin_start = requested_start_sample as f64
                    + duration_samples * (i as f64 / target_peaks as f64);
                let bin_end = requested_start_sample as f64
                    + duration_samples * ((i + 1) as f64 / target_peaks as f64);
                let bin_width = (bin_end - bin_start).max(0.0);

                if bin_width == 0.0 {
                    aligned.push(PeakWindow { max: 0.0, min: 0.0 });
                    continue;
                }

                let clamped_bin_start = bin_start.max(0.0).min(total_samples);
                let clamped_bin_end = bin_end.max(0.0).min(total_samples);
                if clamped_bin_end <= clamped_bin_start {
                    aligned.push(PeakWindow { max: 0.0, min: 0.0 });
                    continue;
                }

                let first_peak = (clamped_bin_start / samples_per_peak).floor() as u64;
                let last_peak_exclusive = (clamped_bin_end / samples_per_peak).ceil() as u64;

                let mut sum_max = 0.0_f64;
                let mut sum_min = 0.0_f64;

                for peak_idx in first_peak..last_peak_exclusive {
                    if peak_idx < start_peak || peak_idx >= end_peak {
                        continue;
                    }

                    let peak_start = peak_idx as f64 * samples_per_peak;
                    let peak_end = peak_start + samples_per_peak;
                    let overlap_start = clamped_bin_start.max(peak_start);
                    let overlap_end = clamped_bin_end.min(peak_end);
                    let overlap = overlap_end - overlap_start;
                    if overlap <= 0.0 {
                        continue;
                    }

                    let local_idx = (peak_idx - start_peak) as usize;
                    if let Some(peak) = channel.get(local_idx) {
                        sum_max += f64::from(peak.max) * overlap;
                        sum_min += f64::from(peak.min) * overlap;
                    }
                }

                aligned.push(PeakWindow {
                    max: (sum_max / bin_width) as f32,
                    min: (sum_min / bin_width) as f32,
                });
            }
            aligned
        })
        .collect();

    PeaksData {
        sample_rate: peaks.sample_rate,
        window_size: peaks.window_size,
        channels,
    }
}

fn total_samples(header: &Header) -> u64 {
    header
        .peak_count
        .saturating_mul(u64::from(header.window_size))
}

fn downsample_peaks(peaks: &mut PeaksData, target_peaks: usize) {
    if peaks.channels.is_empty() {
        return;
    }

    let existing_peaks = peaks.channels[0].len();
    if existing_peaks <= target_peaks {
        return;
    }

    for channel in &mut peaks.channels {
        *channel = average_reduce_channel(channel, target_peaks);
    }
}

fn average_reduce_channel(channel: &[PeakWindow], target_peaks: usize) -> Vec<PeakWindow> {
    let source_len = channel.len();
    if source_len <= target_peaks {
        return channel.to_vec();
    }

    let mut reduced = Vec::with_capacity(target_peaks);
    for i in 0..target_peaks {
        let start = i * source_len / target_peaks;
        let end = ((i + 1) * source_len / target_peaks).max(start + 1);
        let window = &channel[start..end.min(source_len)];

        let mut sum_max = 0.0_f32;
        let mut sum_min = 0.0_f32;
        for peak in window {
            sum_max += peak.max;
            sum_min += peak.min;
        }
        let count = window.len() as f32;
        reduced.push(PeakWindow {
            max: sum_max / count,
            min: sum_min / count,
        });
    }

    reduced
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

    #[test]
    fn reads_with_options_channel_limit_and_reduction() {
        let path = test_file_path();
        let data = PeaksData {
            sample_rate: 20,
            window_size: 1,
            channels: vec![
                vec![
                    PeakWindow {
                        max: 1.0,
                        min: -1.0,
                    },
                    PeakWindow {
                        max: 3.0,
                        min: -3.0,
                    },
                    PeakWindow {
                        max: 5.0,
                        min: -5.0,
                    },
                    PeakWindow {
                        max: 7.0,
                        min: -7.0,
                    },
                ],
                vec![
                    PeakWindow {
                        max: 10.0,
                        min: -10.0,
                    },
                    PeakWindow {
                        max: 20.0,
                        min: -20.0,
                    },
                    PeakWindow {
                        max: 30.0,
                        min: -30.0,
                    },
                    PeakWindow {
                        max: 40.0,
                        min: -40.0,
                    },
                ],
            ],
        };

        write_peaks_file(path.to_str().unwrap(), &data).expect("write");
        let slice = read_peaks_with_options(
            path.to_str().unwrap(),
            &GetPeaksOptions {
                start_seconds: Some(0.0),
                end_seconds: Some(0.2),
                target_peaks: Some(2),
                channels: Some(1),
            },
        )
        .expect("read with options");

        assert_eq!(slice.channels.len(), 1);
        assert_eq!(slice.channels[0].len(), 2);
        assert_eq!(slice.channels[0][0].max, 2.0); // average of 1.0 and 3.0
        assert_eq!(slice.channels[0][1].max, 6.0); // average of 5.0 and 7.0

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn returns_all_when_target_larger_than_available() {
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
            ]],
        };

        write_peaks_file(path.to_str().unwrap(), &data).expect("write");
        let slice = read_peaks_with_options(
            path.to_str().unwrap(),
            &GetPeaksOptions {
                start_seconds: Some(0.0),
                end_seconds: Some(1.0),
                target_peaks: Some(10),
                channels: Some(1),
            },
        )
        .expect("read with options");

        assert_eq!(slice.channels.len(), 1);
        assert_eq!(slice.channels[0].len(), 10);
        assert_eq!(slice.channels[0][0].max, 1.0);
        assert_eq!(slice.channels[0][1].max, 1.0);
        assert_eq!(slice.channels[0][2].max, 2.0);
        assert_eq!(slice.channels[0][3].max, 2.0);
        assert_eq!(slice.channels[0][4].max, 0.0);
        assert_eq!(slice.channels[0][9].max, 0.0);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn zero_pads_when_requested_range_is_beyond_audio() {
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
            ]],
        };

        write_peaks_file(path.to_str().unwrap(), &data).expect("write");
        let slice = read_peaks_with_options(
            path.to_str().unwrap(),
            &GetPeaksOptions {
                start_seconds: Some(1.0),
                end_seconds: Some(2.0),
                target_peaks: Some(4),
                channels: Some(1),
            },
        )
        .expect("read with options");

        assert_eq!(slice.channels.len(), 1);
        assert_eq!(slice.channels[0].len(), 4);
        for peak in &slice.channels[0] {
            assert_eq!(peak.max, 0.0);
            assert_eq!(peak.min, 0.0);
        }

        let _ = std::fs::remove_file(path);
    }
}
