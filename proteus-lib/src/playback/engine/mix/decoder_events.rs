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
