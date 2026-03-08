//! Decoded packet events produced by source decoders and consumed by the mix router.

use super::buffer_mixer::SourceKey;

/// One decoded packet emitted by a decode worker.
#[derive(Debug, Clone)]
pub(crate) struct DecodedPacket {
    /// Source identity for routing.
    pub(crate) source_key: SourceKey,
    /// Packet timestamp in source/playback timeline seconds.
    pub(crate) packet_ts: f64,
    /// Interleaved PCM samples.
    pub(crate) samples: Vec<f32>,
    /// End-of-stream signal for this source.
    pub(crate) eos_flag: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoded_packet_stores_fields() {
        let packet = DecodedPacket {
            source_key: SourceKey::TrackId(7),
            packet_ts: 1.25,
            samples: vec![0.1, -0.1],
            eos_flag: true,
        };
        assert!(matches!(packet.source_key, SourceKey::TrackId(7)));
        assert_eq!(packet.packet_ts, 1.25);
        assert_eq!(packet.samples.len(), 2);
        assert!(packet.eos_flag);
    }
}
