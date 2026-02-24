# Playback Alignment Notes

## Mixing Alignment Rule
- The mixer waits for startup buffering: all active track buffers must reach `start_buffer_ms` before playback starts.
- During steady-state mixing, only emit audio when all active (non-finished) tracks have at least `min_mix_samples` buffered.
- The current implementation lives under `proteus-lib/src/playback/engine/mix/buffer_mixer/` and `.../mix/runner/` (the old single-file `mix.rs` structure has been split).
- If a track is marked finished, the mixer is allowed to drain its remaining buffered samples without blocking on it.
- Mixing while a non-finished track buffer is empty causes dropouts and misalignment; avoid advancing the playhead in that case.
- Reverb/effects tail is mixed only after track buffers are empty, so keep the effects buffer draining logic intact.
- Diffusion reverb implementation notes and tuning guidance live in `.guides/diffusion_reverb.md`.
- Diffusion reverb now uses per-channel decorrelated lanes; avoid reverting to a single shared interleaved delay network, which increases metallic ringing.
- File-based playback preserves alignment by advancing non-active windows with underlay silence.
  The silence is now represented with virtual zero-fill segments (metadata) instead of materialized zero sample buffers.

## Track End Detection
- Container playback marks tracks finished in two ways:
  - If codec duration is known and packet timestamps reach the end.
  - If `track_eos_ms` elapses without packets for a track (default 1000ms).
- The end-of-stream heuristic is required so the mixer can finish draining when duration metadata is missing.

## Startup / Seek Performance Constraints (Alignment-Safe)
- File-based startup now uses a two-phase decode policy in backpressure:
  - startup phase favors instances/sources that are behind the startup target
  - steady-state returns to normal buffering behavior after the start gate passes
- This optimization is alignment-safe because it only changes decode scheduling priority, not timeline semantics.
- Do not “optimize” startup by skipping zero-fill/alignment advancement for future instances unless you can prove exact sample alignment is preserved.

## Debugging Approach
- When diagnosing user-reported playback/UI issues, propose and run targeted tests to validate the hypothesis before implementing a fix, unless the root cause is already unambiguous.
- Useful CLI checks:
  - `--read-durations` vs `--scan-durations` to compare metadata vs packet-derived durations.
  - `--start-buffer-ms` and `--track-eos-ms` to validate buffer/finish behavior.
  - `--features debug` plus `RUST_LOG=debug` for mixer and reverb metrics.
  - Inspect `play trace` `gate_wait_ms=...` to isolate resume/start gate delay from total command-relative time.
