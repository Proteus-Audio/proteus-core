//! Binary read and write operations for `.peaks` files.

use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};

use super::super::{PeakWindow, PeaksData, PeaksError};
use super::header::{write_header, Header, HEADER_SIZE, PEAK_BYTES_PER_CHANNEL};

pub(in crate::peaks) fn write_peaks_file(path: &str, peaks: &PeaksData) -> Result<(), PeaksError> {
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

pub(super) fn read_peaks_by_indices<R: Read + Seek>(
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
