//! Binary header for the `.peaks` file format.

use std::io::{Read, Write};

use super::super::PeaksError;

pub(super) const MAGIC: [u8; 8] = *b"PPEAKS01";
pub(super) const VERSION: u16 = 1;
pub(super) const HEADER_SIZE: u64 = 64;
pub(super) const HEADER_BYTES_USED: usize = 36;
pub(super) const PEAK_BYTES_PER_CHANNEL: u64 = 8; // max f32 + min f32

pub(super) struct Header {
    pub(super) channels: u16,
    pub(super) sample_rate: u32,
    pub(super) window_size: u32,
    pub(super) peak_count: u64,
    pub(super) data_offset: u64,
}

pub(super) fn write_header<W: Write>(writer: &mut W, header: &Header) -> Result<(), PeaksError> {
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

pub(super) fn read_header<R: Read>(reader: &mut R) -> Result<Header, PeaksError> {
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
