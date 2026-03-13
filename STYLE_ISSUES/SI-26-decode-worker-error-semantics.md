# SI-26: Decode Workers Still Conflate Stream Exhaustion With Real Packet Errors

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/playback/engine/mix/runner/decode/file_worker.rs` | `format.next_packet()` errors still break the loop without distinguishing EOF from real failures |
| `proteus-lib/src/playback/engine/mix/runner/decode/container_worker.rs` | Any `next_packet()` error still emits `StreamExhausted`, even when the error is not true EOF |

---

## Current state

Per-packet decode errors are now surfaced through `DecodeWorkerEvent::SourceError`, but
demux/packet-read errors from `format.next_packet()` are still too coarse:

- file worker: `Err(_) => break`
- container worker: `Err(_) => { send StreamExhausted; break }`

### Why this matters

- I/O failure and malformed-stream failure should not masquerade as normal end-of-stream
- Container worker behavior can still over-broadcast normal completion semantics on real errors
- Error semantics drive mixer shutdown and per-source completion behavior

### Recommended remediation

1. Match specifically on Symphonia's end-of-stream vs real-error variants instead of collapsing all
   packet-read errors
2. Send `StreamExhausted` only for true EOF
3. Emit `SourceError` with context for real demux/read failures before terminating that source
4. Revisit whether a single source failure should end only that source instead of acting like a
   global stream completion event
5. Add tests for:
   - true EOF
   - recoverable decode error
   - fatal packet-read/demux error

### Acceptance criteria

- [ ] `next_packet()` EOF is distinguished from real packet-read errors
- [ ] `StreamExhausted` is sent only for true end-of-stream conditions
- [ ] Fatal demux/read failures produce `SourceError` with context
- [ ] Tests cover EOF vs real-error behavior in both file and container workers

## Status

Open.
