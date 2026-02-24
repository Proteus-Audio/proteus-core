# Startup Performance Notes (Current)

This file tracks startup/seek performance work that has already landed, plus the highest-value remaining opportunities.

## Implemented Wins

### Playback output stream reuse (large win)
- `Player` now keeps a persistent `OutputStream` and reuses its mixer across playback thread restarts.
- This removes repeated OS audio device open costs on subsequent starts/seeks/reinitializations.
- Result: the old `~130ms` `output stream opened` cost no longer appears on the hot path for most repeat playback actions.

### Duplicate container metadata scan removed
- Container playback now reuses `prot.info` instead of recomputing `Info::new(...)` in `Player::new_from_path_or_paths()`.
- This avoids an extra duration scan/probe during startup.

### Effect warmup path now uses `warm_up()` (lower startup overhead)
- Mix startup warmup no longer runs `effect.process(...)` on a zero buffer.
- It calls `effect.warm_up(...)` directly, which preserves first-chunk readiness while avoiding unnecessary startup DSP work.

### Convolution IR caching (decoded IR + kernel/template cache)
- Convolution reverb now caches decoded impulse responses.
- It also caches a prebuilt convolution reverb template/kernel state and clones/reset it for reuse.
- Result: repeated starts/seeks with the same IR are noticeably faster.

### File-mode startup decode prioritization (two-phase behavior)
- During file-based startup only, decode backpressure temporarily prioritizes sources/instances with startup deficits.
- Once the start gate passes, buffering returns to normal behavior.
- This reduces startup delay caused by some file workers over-buffering while others lag.

### Virtual zero-fill in `BufferMixer` (large file-mode win)
- Alignment-preserving silence spans are now stored as metadata segments instead of materialized zero samples.
- This keeps exact track alignment semantics while reducing queue churn and allocations.
- Result: file-based startup/seek can reach first output much faster when many instances need underlay silence.

## Important Alignment Invariants (Do Not Break)

- Logical track/instance alignment is preserved by maintaining equal-length buffered timelines across active instances.
- Non-active windows still advance via underlay silence (now represented virtually as zero segments).
- Do not optimize by skipping alignment advancement for future/inactive instances unless the replacement preserves timeline equivalence exactly.

## Remaining Opportunities

### `Prot::load_play_settings()` reparses the container
- Container playback settings loading still reopens/reparses Matroska to read `play_settings.json`.
- Reusing an already-open handle or cached attachment bytes could reduce startup work before playback begins.

### Overlap mixer startup with device setup (if compatible with current threading constraints)
- Earlier guidance suggested starting the engine receiver/mix work before device open to overlap work.
- Persistent stream reuse reduced the urgency, but overlap can still help on first-run cold starts.
- Any change here must respect platform/thread constraints around audio stream ownership (especially macOS/CoreAudio).

### Startup/seek instrumentation polish
- Keep using `gate_wait_ms` in play trace logs to separate resume-gate time from the absolute `+...ms` command-relative timer.
- Additional targeted per-stage timers may still help identify regressions quickly.

## Notes for Future Tuning

- For convolution-heavy projects, first use of a new IR may still be slower than subsequent uses (cache fill).
- For file-based playback, if startup latency regresses, inspect:
  - startup fairness/backpressure behavior
  - virtual zero-fill usage (silence should not be materialized eagerly)
  - start gate thresholds (`start_samples`, `min_mix_samples`)
