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
}

/// Structured event stream emitted by decode workers.
#[derive(Debug, Clone)]
pub(crate) enum DecodeWorkerEvent {
    /// Decoded PCM packet ready for routing.
    Packet(DecodedPacket),
    /// Source reached end-of-stream.
    SourceFinished { source_key: SourceKey },
    /// Source emitted a decode/runtime issue.
    SourceError {
        source_key: SourceKey,
        recoverable: bool,
        message: String,
    },
    /// Shared stream exhausted (for example, container demux EOF).
    StreamExhausted,
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
        };
        assert!(matches!(packet.source_key, SourceKey::TrackId(7)));
        assert_eq!(packet.packet_ts, 1.25);
        assert_eq!(packet.samples.len(), 2);
    }

    #[test]
    fn decode_worker_event_tracks_error_kind() {
        let event = DecodeWorkerEvent::SourceError {
            source_key: SourceKey::TrackId(3),
            recoverable: true,
            message: "decode glitch".to_string(),
        };
        assert!(matches!(
            event,
            DecodeWorkerEvent::SourceError {
                recoverable: true,
                ..
            }
        ));
    }
}
