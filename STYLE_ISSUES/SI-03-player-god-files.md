# SI-03: Player — Oversized Files

## Files affected

| File | Lines |
|---|---|
| `proteus-lib/src/playback/player/runtime/worker/runner.rs` | 893 |
| `proteus-lib/src/playback/player/mod.rs` | 617 |
| `proteus-lib/src/playback/player/controls.rs` | 469 |

---

## `worker/runner.rs` (893 lines)

### Current structure

The file has good function-level decomposition (most functions are ≤40 lines). The problem
is density: 30+ private helper functions and a large test module all live in one file.
The helpers naturally group into four concerns:

1. **Loop lifecycle** — `run_playback_thread`, `run_engine_receive_loop`,
   `run_drain_loop`, `LoopState` (lines 24–186)
2. **Sink management** — `initialize_sink`, `pause_sink`, `resume_sink`,
   `append_startup_silence`, `wait_for_sink_capacity`, `update_sink` (lines 235–697)
3. **State checks and transitions** — `check_runtime_state`, `handle_abort`,
   `handle_resuming_gate`, `handle_pausing`, `handle_resuming_commit`,
   `apply_end_of_stream_action` (lines 369–802)
4. **Timing and diagnostics** — `advance_playback_clock`, `update_chunk_lengths`,
   `update_append_timing`, `mark_buffering_complete`, `is_drain_complete`,
   `play_trace_elapsed_ms`, `log_drain_loop_start` (lines 491–822)

### Proposed split

```
runtime/worker/
├── runner.rs       # Entry points: run_playback_thread, run_engine_receive_loop,
│                   # run_drain_loop, LoopState + impl — ~200 lines
├── sink.rs         # initialize_sink, pause_sink, resume_sink, append_startup_silence,
│                   # wait_for_sink_capacity, update_sink — ~250 lines
├── transitions.rs  # check_runtime_state, handle_abort, handle_resuming_gate,
│                   # handle_pausing, handle_resuming_commit,
│                   # apply_end_of_stream_action — ~200 lines
└── timing.rs       # advance_playback_clock, update_chunk_lengths, update_append_timing,
                    # mark_buffering_complete, is_drain_complete,
                    # play_trace_elapsed_ms, log_drain_loop_start — ~200 lines
```

Tests should be distributed into each sub-file next to the functions they test.

**Expected result**: each file ≤250 lines.

---

## `player/mod.rs` (617 lines)

### Current structure

`mod.rs` already delegates to focused submodules (`controls`, `effects`, `settings`,
`runtime`) but still contains:

- `Player` struct definition with ~20 `Arc`/`Mutex` fields (lines 37–100)
- Two large constructors: `try_from_source_with_options` (~79 lines, l.246) and
  helper `build_from_source` (lines 130–245)
- `Drop` impl — 74 lines of cleanup logic (lines 550–624)
- Forwarding methods that delegate to submodules (lines 324–549)

### Proposed changes

1. Extract the `Drop` cleanup logic into `controls.rs` as a private `drop_cleanup`
   helper, and keep the `Drop` impl in `mod.rs` as a one-liner:
   ```rust
   impl Drop for Player {
       fn drop(&mut self) {
           controls::drop_cleanup(self);
       }
   }
   ```
2. Extract `try_from_source_with_options` and `build_from_source` into a new
   `builder.rs` sub-module. These functions are purely construction logic and
   have no reason to live alongside the forwarding API surface.

```
player/
├── mod.rs       # Player struct, forwarding methods, Drop impl (thin) — ~300 lines
├── builder.rs   # try_from_source_with_options, build_from_source — ~200 lines
├── controls.rs  # (+ drop_cleanup) — ~250 lines
├── effects.rs
├── settings.rs
└── runtime/
```

**Expected result**: `mod.rs` ≤300 lines, `builder.rs` ~200 lines.

---

## `player/controls.rs` (469 lines)

### Current structure

`controls.rs` mixes three concerns:

1. **Transport controls** — `play`, `pause`, `stop`, `seek`, `fade_in`, `fade_out`
   (lines 1–180)
2. **Thread lifecycle** — `start_playback_thread`, `stop_and_join_playback_thread`,
   `join_playback_thread` (lines 180–250)
3. **State reads** — `current_ms`, `state`, `is_alive`, `stop_resets_timestamp`
   and related helpers (lines 250–469)

### Proposed split

```
player/
├── controls.rs      # Transport controls (play/pause/stop/seek/fade) — ~180 lines
├── lifecycle.rs     # Thread start/stop/join, drop cleanup — ~150 lines
└── state.rs         # State reads, timestamp queries, is_alive — ~150 lines
```

Alternatively, if the split feels too granular, merge `lifecycle.rs` back into
`controls.rs` and extract only the state-read helpers into `state.rs`.

**Expected result**: each file ≤250 lines.

---

## Acceptance criteria

- [ ] All existing tests pass (`cargo test -p proteus-lib`)
- [ ] `cargo check --all-features` shows no new errors or warnings
- [ ] Each new file is ≤400 lines
- [ ] `pub(in crate::playback::player)` visibility paths remain valid after restructuring
- [ ] The `Drop` impl compiles and the player still cleans up correctly under test
