# SI-30: Standalone and Container Decode Paths Still Implement EOS Logic Independently

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/playback/engine/mix/runner/decode/mod.rs` | Shared decode helpers: `decode_and_forward_packet`, `interleaved_samples`, `packet_ts_seconds`, `forward_decoded_packet` |
| `proteus-lib/src/playback/engine/mix/runner/decode/file_worker.rs` | Standalone file decode worker (uses shared helpers) |
| `proteus-lib/src/playback/engine/mix/runner/decode/container_worker.rs` | Container demux decode worker (uses shared helpers) |
| `proteus-lib/src/track/` | Legacy decode paths (superseded, no longer called by active code) |

---

## Current state

The shared decode → interleave → forward pipeline is factored into `decode_and_forward_packet`
in the shared `decode/mod.rs` module. Both file and container workers delegate to this single
function for packet decoding, error classification, and forwarding, ensuring identical behavior
for:

- Successful decode: interleave to stereo, apply backpressure, forward to mixer.
- Recoverable decode errors (`DecodeError`): report as recoverable, continue.
- Fatal decode errors: report as non-recoverable, stop.

### Intentional mode-specific differences

| Behavior | File worker | Container worker | Reason |
|---|---|---|---|
| Stream exhaustion signal | `SourceFinished` only | `StreamExhausted` + per-track `SourceFinished` | Container EOF affects all embedded tracks simultaneously |
| Source key type | `SourceKey::FilePath` | `SourceKey::TrackId` | Different source identification models |
| Seek failure | Recoverable error, continue from start | Recoverable error, continue from start | Same behavior (parity) |

### Acceptance criteria

- [x] Shared EOS and shutdown behavior is factored into common helpers where appropriate
- [x] Intentional differences between standalone and container decode paths are documented
- [x] Tests cover parity of common completion scenarios across both modes

## Status

Resolved.
