use super::{MuxError, Result};

const US_PER_SECOND: u64 = 1_000_000;
const DEFAULT_PACKET_SAMPLES: u64 = 1024;

#[derive(Debug)]
pub(super) struct OggPacket {
    pub(super) data: Vec<u8>,
    end_sample: Option<u64>,
}

#[derive(Debug)]
pub(super) struct TimedPacket {
    pub(super) timestamp: u64,
    pub(super) duration: Option<u64>,
    pub(super) payload: Vec<u8>,
}

pub(super) fn read_ogg_packets(bytes: &[u8], label: &str) -> Result<Vec<OggPacket>> {
    let mut offset = 0usize;
    let mut pending = Vec::new();
    let mut packets = Vec::new();
    let mut stream_serial = None;

    while offset < bytes.len() {
        if bytes.get(offset..offset + 4) != Some(b"OggS") {
            return Err(MuxError::InvalidInput(format!(
                "{label} is not an Ogg stream"
            )));
        }
        let version = bytes
            .get(offset + 4)
            .copied()
            .ok_or_else(|| MuxError::InvalidInput("truncated Ogg page".to_string()))?;
        if version != 0 {
            return Err(MuxError::InvalidInput(format!(
                "{label} uses unsupported Ogg stream version {version}"
            )));
        }

        let granule = read_le_i64(bytes, offset + 6)
            .ok_or_else(|| MuxError::InvalidInput("truncated Ogg page granule".to_string()))?;
        let page_serial = read_le_u32(bytes, offset + 14)
            .ok_or_else(|| MuxError::InvalidInput("truncated Ogg page serial".to_string()))?;
        if let Some(stream_serial) = stream_serial {
            if page_serial != stream_serial {
                return Err(MuxError::InvalidInput(format!(
                    "{label} contains multiple Ogg logical streams; pass each stream separately"
                )));
            }
        } else {
            stream_serial = Some(page_serial);
        }

        let segment_count = usize::from(
            *bytes
                .get(offset + 26)
                .ok_or_else(|| MuxError::InvalidInput("truncated Ogg page".to_string()))?,
        );
        let segment_table_start = offset + 27;
        let body_start = segment_table_start + segment_count;
        if body_start > bytes.len() {
            return Err(MuxError::InvalidInput(
                "truncated Ogg segment table".to_string(),
            ));
        }

        let segment_table = &bytes[segment_table_start..body_start];
        let body_len: usize = segment_table
            .iter()
            .map(|segment| usize::from(*segment))
            .sum();
        let body_end = body_start + body_len;
        if body_end > bytes.len() {
            return Err(MuxError::InvalidInput(
                "truncated Ogg page body".to_string(),
            ));
        }

        let mut completed_on_page = Vec::new();
        let mut body_offset = body_start;
        for segment_len in segment_table {
            let segment_len = usize::from(*segment_len);
            pending.extend_from_slice(&bytes[body_offset..body_offset + segment_len]);
            body_offset += segment_len;

            if segment_len < 255 {
                completed_on_page.push(OggPacket {
                    data: std::mem::take(&mut pending),
                    end_sample: None,
                });
            }
        }

        if granule >= 0 {
            if let Some(packet) = completed_on_page.last_mut() {
                packet.end_sample = Some(u64::try_from(granule)?);
            }
        }
        packets.extend(completed_on_page);
        offset = body_end;
    }

    if !pending.is_empty() {
        return Err(MuxError::InvalidInput(
            "Ogg stream ended with an incomplete packet".to_string(),
        ));
    }

    Ok(packets)
}

pub(super) fn time_packets(packets: &[OggPacket], sample_rate: u32) -> Vec<TimedPacket> {
    let mut timed = Vec::with_capacity(packets.len());
    let mut segment_start = 0usize;
    let mut segment_start_sample = 0u64;

    for (index, packet) in packets.iter().enumerate() {
        if let Some(end_sample) = packet.end_sample {
            let count = u64::try_from(index - segment_start + 1).unwrap_or(1);
            let sample_span = end_sample.saturating_sub(segment_start_sample);
            let step = (sample_span / count).max(1);

            for (segment_index, packet) in packets[segment_start..=index].iter().enumerate() {
                let start_sample =
                    segment_start_sample + step * u64::try_from(segment_index).unwrap_or(0);
                let next_sample = if segment_index + 1 == usize::try_from(count).unwrap_or(0) {
                    end_sample
                } else {
                    start_sample + step
                };
                timed.push(TimedPacket {
                    timestamp: samples_to_us(start_sample, sample_rate),
                    duration: Some(
                        samples_to_us(next_sample, sample_rate)
                            .saturating_sub(samples_to_us(start_sample, sample_rate)),
                    ),
                    payload: packet.data.clone(),
                });
            }

            segment_start = index + 1;
            segment_start_sample = end_sample;
        }
    }

    for (offset, packet) in packets[segment_start..].iter().enumerate() {
        let start_sample =
            segment_start_sample + DEFAULT_PACKET_SAMPLES * u64::try_from(offset).unwrap_or(0);
        timed.push(TimedPacket {
            timestamp: samples_to_us(start_sample, sample_rate),
            duration: Some(samples_to_us(DEFAULT_PACKET_SAMPLES, sample_rate)),
            payload: packet.data.clone(),
        });
    }

    timed
}

pub(super) fn read_le_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes
        .get(offset..offset + 4)
        .map(|value| u32::from_le_bytes(value.try_into().expect("slice length checked")))
}

fn read_le_i64(bytes: &[u8], offset: usize) -> Option<i64> {
    bytes
        .get(offset..offset + 8)
        .map(|value| i64::from_le_bytes(value.try_into().expect("slice length checked")))
}

fn samples_to_us(samples: u64, sample_rate: u32) -> u64 {
    if sample_rate == 0 {
        return 0;
    }

    ((u128::from(samples) * u128::from(US_PER_SECOND)) / u128::from(sample_rate)) as u64
}
