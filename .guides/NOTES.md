# Playback Alignment Notes

## Mixing Alignment Rule
- The mixer waits for startup buffering: all active track buffers must reach `start_buffer_ms` before playback starts.
- During steady-state mixing, only emit audio when all active (non-finished) tracks have at least `min_mix_samples` buffered
  (see `MIN_MIX_MS` in `proteus-lib/src/playback/engine/mix.rs`).
- If a track is marked finished, the mixer is allowed to drain its remaining buffered samples without blocking on it.
- Mixing while a non-finished track buffer is empty causes dropouts and misalignment; avoid advancing the playhead in that case.
- Reverb/effects tail is mixed only after track buffers are empty, so keep the effects buffer draining logic intact.

## Track End Detection
- Container playback marks tracks finished in two ways:
  - If codec duration is known and packet timestamps reach the end.
  - If `track_eos_ms` elapses without packets for a track (default 1000ms).
- The end-of-stream heuristic is required so the mixer can finish draining when duration metadata is missing.

## Debugging Approach
- When diagnosing user-reported playback/UI issues, propose and run targeted tests to validate the hypothesis before implementing a fix, unless the root cause is already unambiguous.
- Useful CLI checks:
  - `--read-durations` vs `--scan-durations` to compare metadata vs packet-derived durations.
  - `--start-buffer-ms` and `--track-eos-ms` to validate buffer/finish behavior.
  - `--features debug` plus `RUST_LOG=debug` for mixer and reverb metrics.
