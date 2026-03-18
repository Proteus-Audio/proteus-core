# SI-35 — Residual functions exceeding the 80-line hard limit

## Rule

Style Guide §3 (File and Function Size Limits): functions have a **recommended** limit
of 40 lines and a **hard limit** of 80 lines.

## Problem

Three non-test functions still exceed the 80-line hard limit:

| File | Function | Lines | Notes |
|---|---|---|---|
| `track/single.rs` | `buffer_track` | ~102 | Reduced from ~165 in SI-05; most length is the `thread::spawn` closure |
| `playback/player/runtime/thread.rs` | `initialize_thread` | ~100 | ~30 lines are `ThreadContext` struct construction |
| `logging/pivot_buffer_trace.rs` | `pivot_buffer` | ~157 | Feature-gated `debug` utility, not production code |

Additionally, 4 `#[allow(clippy::too_many_arguments)]` suppressions remain from SI-11:

| File | Function | Params |
|---|---|---|
| `track/single.rs` | `process_decoded_packet` | 8 |
| `track/container.rs` | `check_eos_skew` | 7 |
| `track/container.rs` | `push_decoded_container_packet` | 8 |
| `track/container.rs` | `run_container_decode_loop` | 8 |

### Why this matters

- Long functions are harder to test, review, and reason about
- The style guide hard limit exists to enforce this
- The `too_many_arguments` suppressions paper over the lint instead of fixing the design

### Recommended remediation

1. **`buffer_track`**: Extract the decode-loop body and error-handling branches from the
   `thread::spawn` closure into named helpers
2. **`initialize_thread`**: Extract output-stream setup and `ThreadContext` construction
   into helpers
3. **`pivot_buffer`**: Lower priority since it is debug-only; extract parsing and
   formatting phases into helpers
4. **Track helpers**: Introduce a shared `TrackDecodeContext` struct bundling the
   `buffer_map`, `buffer_notify`, and `abort` parameters shared across all four functions

### Acceptance criteria

- [ ] All three functions are ≤ 80 lines
- [ ] All 4 `#[allow(clippy::too_many_arguments)]` suppressions in `track/` are removed
- [ ] Tests continue to pass
- [ ] No new `#[allow(...)]` suppressions introduced

## Status

Open.
