# SI-02: Mix Engine — Oversized Files

## Files affected

| File | Lines |
|---|---|
| `proteus-lib/src/playback/engine/mix/buffer_mixer/mod.rs` | 944 |
| `proteus-lib/src/playback/engine/mix/runner/mod.rs` | 715 |

---

## `buffer_mixer/mod.rs` (944 lines)

### Current structure

The file already has sub-files (`aligned_buffer`, `backpressure`, `routing_helpers`,
`routing_time`) but the `mod.rs` itself still contains 944 lines, nearly all of it the
`BufferMixer` impl block (lines 125–814) plus a test module (lines 815–944).

The `BufferMixer` impl has two distinct responsibilities:

1. **Mutation** — `new`, `write_section`, `pivot` (the real-time write path)
2. **Inspection** — `track_ready`, `instance_buffer_fills`, `tracks_fill_state`,
   `track_fill_state`, `mix_fill_state`, `counters` (diagnostic/debug reads)

Additionally the `RouteDecision` logic and `SourceKey` type sit in `mod.rs` when they
belong in `routing_helpers.rs`.

### Proposed changes

1. Move `SourceKey`, `RouteDecision`, and `FillState` into `routing_helpers.rs`
   (they are purely data types used by routing logic).
2. Extract the diagnostic/inspection methods into a new `diagnostics.rs` sub-file:
   ```
   buffer_mixer/
   ├── mod.rs           # BufferMixer struct, new(), write_section(), pivot() — ~400 lines
   ├── aligned_buffer.rs
   ├── backpressure.rs
   ├── diagnostics.rs   # track_ready, *_fill_state, counters, instance_buffer_fills
   ├── routing_helpers.rs  # (+ SourceKey, RouteDecision, FillState moved here)
   └── routing_time.rs
   ```

**Expected result**: `mod.rs` ≤400 lines, `diagnostics.rs` ~150 lines.

---

## `runner/mod.rs` (715 lines)

### Current structure

The mix runner contains the main decode loop plus supporting state types:

- `MixState`, `TransitionState`, and supporting enums (lines 1–80)
- `RunnerConfig` and `MixRunner` struct + impl (lines 80–220)
- Private loop body helpers: `advance`, `mix_section`, `pivot_buffer`,
  `check_transition`, `apply_inline_updates`, and many smaller helpers (lines 220–680)
- Tests (lines 680–715)

### Proposed split

The loop-body helpers form a coherent group that can move to a new file:

```
runner/
├── mod.rs        # MixState, MixRunner, RunnerConfig, public API — ~300 lines
└── loop_body.rs  # advance, mix_section, pivot_buffer, check_transition,
                  # apply_inline_updates, and their private helpers — ~400 lines
```

Alternatively, extract just the transition logic (which is already somewhat self-contained
around `TransitionState`) into a `transition.rs` to keep all files under 400 lines.

**Expected result**: each file ≤400 lines.

---

## Acceptance criteria

- [x] All existing tests pass (`cargo test -p proteus-lib`)
- [x] `cargo check --all-features` shows no new errors or warnings
- [x] Each new file is ≤400 lines
- [x] No public import paths used by callers are broken (add re-exports as needed)
- [x] `pub(crate)` visibility is preserved; no accidental widening
