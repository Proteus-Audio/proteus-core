# proteus-lib Improvement Roadmap

This roadmap consolidates findings from `STATUS_REPORT.md` and `STATUS_REPORT_COMPARE.md` into a prioritized plan. Items are grouped into four phases by severity. Each phase should be completed and verified with `cargo check`, `cargo clippy`, and `cargo test` before starting the next.

---

## Phase 1 — Critical: Production Panics and Correctness Bugs

These items can crash a host application on ordinary user input. They have no acceptable workaround and should be fixed before any other work.

### 1.1 Replace panics in `container/info.rs` `reduce_track_infos` with errors

**Files:** `proteus-lib/src/container/info.rs` lines 471, 478, 488, 512

The function panics on mismatched sample rates, channel layouts, and bit depths, then calls `.unwrap()` on the result. All four sites should be converted to return a typed error via `thiserror`. Callers up the chain (`Info::new_from_file_paths`, etc.) should propagate the error.

- [x] Define an error variant for format mismatch (e.g., `InfoError::IncompatibleTracks`)
- [x] Replace `panic!("Sample rates do not match")` at line 471 with `return Err(...)`
- [x] Replace `panic!("Channel layouts do not match ...")` at lines 478–481 with `return Err(...)`
- [x] Replace `panic!("Bits per sample do not match")` at line 488 with `return Err(...)`
- [x] Replace `info.unwrap()` at line 512 with `unwrap_or` (safe after `is_empty` guard)
- [x] Update all callers to handle the new error path
- [x] Add a test case for multi-file input with mismatched sample rates

### 1.2 Fix file-open panic in `container/info.rs` `get_probe_result_from_string`

**File:** `proteus-lib/src/container/info.rs` lines 71–73

The function signature returns `Result` but internally calls `.expect()` on `File::open`. Any missing file, permission error, or transient I/O failure panics instead of returning a recoverable error.

- [x] Replace `File::open(path).expect("failed to open media file")` with `match File::open(path)` returning `Err(Error::IoError(e))`
- [x] Verify all callers already propagate `Result` correctly

### 1.3 Replace `catch_unwind` error-handling in `get_durations_best_effort`

**File:** `proteus-lib/src/container/info.rs` lines 152–162

`catch_unwind` is used because the underlying code panics on expected error conditions. The right fix is to make the underlying probing code return `Result` (a prerequisite of 1.1 and 1.2), at which point `catch_unwind` can be removed entirely.

- [x] After 1.1 and 1.2 are complete, verify that `get_durations_best_effort` no longer needs to catch panics
- [x] Remove the `catch_unwind` call and replace with a direct call to `get_durations`

### 1.4 Fix decoder selection bug in `tools/tools.rs` `get_decoder`

**File:** `proteus-lib/src/tools/tools.rs` lines 64–72

`get_reader` correctly finds the first supported audio track, but `get_decoder` always builds the decoder from `format.tracks()[0]` regardless of which track index was found. If the first track in the container is null or unsupported (e.g., a video or data track), `get_reader` succeeds and `get_decoder` panics on "unsupported codec."

- [x] Pass the selected track index from `get_reader` through to `get_decoder` (or look it up by the same criteria)
- [x] Replace the unconditional `format.tracks()[0]` with a `find(non-null codec)` lookup
- [ ] Add a test with a container where the first track is not the first decodable audio track

### 1.5 Handle missing audio codec gracefully in `track/single.rs`

**File:** `proteus-lib/src/track/single.rs` lines 57–71

`.expect("no track found")` panics if the container probe succeeds but contains no decodable audio track. The decode thread should instead send an error signal through the track's channel so the mix engine can handle it as a failed source rather than crashing.

- [x] Replace `.expect("no track found")` with an error path that marks the track finished and notifies the condvar
- [x] Verify the mix engine handles a failed source without crashing (mix engine already handles finished tracks gracefully)

---

## Phase 2 — High: Correctness Risks and Real-Time Integrity

These issues do not always cause crashes but represent real risks to correct behavior, especially under concurrency or load.

### 2.1 Fix atomic memory ordering on `playback_thread_exists` and `buffering_done`

**Files:** `proteus-lib/src/playback/player/runtime/thread.rs`, `player/settings.rs`, `player/mod.rs`, `worker/runner.rs`

`playback_thread_exists` is written with `SeqCst` but read with `Relaxed` in at least one site. `buffering_done` is both set and cleared with `Relaxed` across different threads with no release/acquire pair. Both are used for cross-thread lifecycle decisions.

- [ ] Audit every load/store of `playback_thread_exists` and choose a consistent ordering (at minimum `Release` on write, `Acquire` on read)
- [ ] Audit every load/store of `buffering_done` and apply `Release`/`Acquire` pairing
- [ ] Add a comment at each atomic documenting the chosen ordering and why

### 2.2 Remove per-chunk `Vec` allocations from the effect-chain hot path

**File:** `proteus-lib/src/playback/engine/mix/effects.rs` lines 17–27

Every effect in the chain allocates a fresh `Vec<f32>` per chunk. For a chain of five effects at 48 kHz stereo this is six allocations per chunk in the hot mixing path.

- [ ] Change `AudioEffect::process` to accept a mutable output slice rather than returning an owned `Vec<f32>` — or pre-allocate two scratch buffers and ping-pong between them
- [ ] Verify that effect tail draining (`drain: bool`) still works correctly with the new interface
- [ ] Benchmark before and after to confirm reduced allocation pressure

### 2.3 Remove `Prot::new` stdout print

**File:** `proteus-lib/src/container/prot.rs` line 83

`println!("Info: {:?}", info)` is called during normal object construction in a library crate. This bypasses the `log` crate, cannot be suppressed by callers, and pollutes host process output.

- [x] Remove the `println!` call (or replace with `log::debug!` if diagnostic value is needed)
- [x] Search the rest of `proteus-lib` for any other `println!`/`eprintln!` calls in non-test code and remove them

### 2.4 Address effects mutex held during full DSP chain execution

**File:** `proteus-lib/src/playback/engine/mix/runner/mod.rs` lines ~482–483

The shared effects mutex is locked and held across the entire `run_effect_chain` call. For convolution reverb this can be expensive. Control/query code in `player/effects.rs` acquires the same mutex, creating a priority inversion risk.

- [ ] Clone the effect chain out of the mutex at the start of each chunk, process outside the lock, then write results back — or use a double-buffered / lock-free handoff
- [ ] Verify that inline effect updates (`set_effects_inline`) still apply correctly with the new handoff strategy

### 2.5 Add poisoning strategy to critical mutex sites

**Files:** Throughout `proteus-lib` (~50 `.lock().unwrap()` calls)

If any thread panics while holding a mutex, all subsequent `.lock().unwrap()` calls on that mutex also panic. This causes cascade failures.

- [ ] For the most critical mutexes (effects chain, player state, sink), replace `.lock().unwrap()` with `.lock().unwrap_or_else(|e| e.into_inner())` or add explicit poisoning recovery
- [x] At minimum, replace all plain `.lock().unwrap()` with `.lock().expect("<context about what this mutex guards>")` so failures produce useful diagnostics

---

## Phase 3 — Medium: Performance and Operational Reliability

These items degrade performance or make failures harder to diagnose. They do not cause crashes under normal conditions but will become problems under load or with unusual input.

### 3.1 Replace polling/sleep loops in the playback worker with tighter signaling

**File:** `proteus-lib/src/playback/player/runtime/worker/runner.rs` lines 115, 510

The worker uses `recv_timeout(Duration::from_millis(20))` as its main loop interval and `thread::sleep(Duration::from_millis(5))` for sink backpressure. These are coarse wakeups that add transport latency and scheduling jitter.

- [ ] Replace the 20ms `recv_timeout` polling loop with a blocking `recv` plus a separate shutdown/control channel for interruption
- [ ] Replace the 5ms sleep backpressure with a condvar or channel-based notification from the sink

### 3.2 Add timeout to `Condvar::wait` in backpressure

**File:** `proteus-lib/src/playback/engine/mix/buffer_mixer/backpressure.rs` line 114

`self.cv.wait(guard)` has no timeout. If the notifying thread exits abnormally without sending a notification, this blocks forever.

- [ ] Replace with `wait_timeout` and check the result; on timeout, check a shutdown flag and return
- [ ] Add a test for the case where the notifying thread exits without notifying

### 3.3 Fix silent seek and decode failures

**File:** `proteus-lib/src/track/single.rs` lines 92, 99–108

Seek failures return silently with no log entry and no signal to the caller. Decode errors within the loop are bound to `_result` and dropped.

- [ ] On seek failure, emit a `log::warn!` with the error and path
- [x] On decode errors within the loop, emit a `log::debug!` or `log::warn!`; distinguish I/O/EOF from true errors
- [ ] Consider whether persistent decode errors should terminate the track (send a finished signal) rather than silently continuing

### 3.4 Fix decode worker error handling — distinguish EOF from real errors

**Files:** `proteus-lib/src/playback/engine/mix/runner/decode/file_worker.rs` lines 71–74, `container_worker.rs` lines 85–90, 169–170

All errors from `format.next_packet()` are treated the same as end-of-stream, including real I/O failures and malformed packets. `container_worker.rs` additionally sends a global `None` signal (finish-all) on certain errors.

- [ ] Match on symphonia's error variants to distinguish `EndOfStream` from real errors
- [ ] Log real errors at `warn` level with context before breaking
- [ ] Consider whether a single-track decode failure should finish only that track rather than triggering the global finish signal

### 3.5 Fix single-container `Info::new` always doing a full-file scan

**File:** `proteus-lib/src/container/info.rs` line 572

`Info::new` calls `get_durations_by_scan` directly, making startup O(file size) even for containers with valid duration metadata. The multi-file path (`new_from_file_paths`) uses best-effort metadata probing first.

- [ ] Change `Info::new` to attempt metadata-based duration reading first (matching the multi-file path)
- [ ] Fall back to `get_durations_by_scan` only when metadata is absent or zero
- [ ] Add a CLI flag note (already documented: `--read-durations` vs `--scan-durations`) so users can force either behavior

### 3.6 Replace O(n) `VecDeque::drain` in premix buffer

**File:** `proteus-lib/src/playback/engine/premix.rs` line 52

`drain(0..take)` on a `VecDeque` is O(n) because it must shift elements. This runs on every emitted chunk.

- [ ] Replace `VecDeque<f32>` with a ring buffer (e.g., `dasp_ring_buffer`, already a dependency) with read/write indices
- [ ] Verify chunk pop semantics are preserved

### 3.7 Pre-compute per-track weights/gains before the inner mixing loop

**File:** `proteus-lib/src/playback/engine/mix/track_mix.rs` lines 82–95

Per-iteration `HashMap::get` calls for track weights and channel gains run on every mix chunk. The active track set is stable within a chunk.

- [ ] At the start of each chunk, snapshot active track weights and channel gains into parallel `Vec`s indexed by position
- [ ] Use index-based access inside the mixing loop

### 3.8 Remove unnecessary clone in `audio/samples.rs` `clone_samples_buffer`

**File:** `proteus-lib/src/audio/samples.rs` lines 19–27

An iterator is collected into a `Vec<f32>` and then immediately cloned again. One of the two copies is unnecessary.

- [ ] Rewrite to produce the `SamplesBuffer` from the first collection without a second clone

### 3.9 Implement track weighting for standalone file playback

**File:** `proteus-lib/src/track/single.rs` line 46

The `_track_weights` parameter is accepted but ignored. Per-track volume control is silently a no-op for standalone file mode.

- [ ] Apply the `track_weights` scalar to samples before pushing to the ring buffer
- [ ] Add a test confirming that a weight of `0.5` produces samples at half amplitude
- [ ] Remove the leading underscore from `_track_weights` once implemented

### 3.10 Fix float-to-`usize` cast in buffer helpers

**File:** `proteus-lib/src/playback/engine/mix/buffer_mixer/helpers.rs` lines 129–132

A `f64` value is floored and cast directly to `usize`. If unexpectedly negative due to floating-point rounding, the behavior is platform-dependent.

- [ ] Add `.max(0.0)` before the `as usize` cast to make the sign guarantee explicit on all platforms

---

## Phase 4 — Code Quality: Maintainability and API Hygiene

These items do not affect correctness today but accumulate technical debt and make future work riskier.

### 4.1 Remove identified dead code

**Files:** Multiple — see below

The crate compiles with a large warning set that makes real regressions harder to notice.

- [x] Remove `ShuffleRuntimePlan` and `build_runtime_shuffle_plan` from `container/prot.rs`
- [x] Remove `OutputStageArgs` and `produce_output_samples` from `playback/engine/mix/output_stage.rs`
- [x] Remove `SourceSpawner` from `playback/engine/mix/source_spawner.rs`
- [x] Remove or activate `add_samples_to_buffer_map_nonblocking` in `track/buffer.rs`
- [x] Fix `track/mod.rs` re-exports that are unused and not re-exported from `lib.rs`
- [ ] Remove unused functions from the internal `logging` module
- [x] Remove unused buffer diagnostics helpers in `playback/engine/mix/buffer_mixer/helpers.rs`
- [x] Remove the commented-out `PartialEq` block in `container/info.rs` lines 211–222
- [ ] Run `cargo clippy` and address all dead code warnings

### 4.2 Split oversized files by responsibility

Files that mix multiple concerns make changes harder to review and test safely. Target: no file over ~400 lines.

- [x] **`container/prot.rs` (1,741 lines):** Extract shuffle schedule generation, runtime planning, track-mix settings, and combination counting into separate submodules
- [x] **`playback/player/runtime/worker/runner.rs` (749 lines):** Extract sink management, timing, start-gate logic, and metrics aggregation into separate modules
- [x] **`playback/engine/mix/buffer_mixer/mod.rs` (846 lines):** Extract instance management and the main mix loop into submodules
- [x] **`container/info.rs` (614 lines):** Separate metadata probing, duration scanning, and format reduction into distinct modules
- [x] **`playback/engine/mix/runner/mod.rs` (650 lines):** Extract decode worker spawning and the main mix loop body

### 4.3 Complete or remove `BasicReverb` deprecation

**File:** `proteus-lib/src/dsp/effects/mod.rs`

The `BasicReverb` variant is deprecated but still matched with `#[allow(deprecated)]` in `process()`, `reset_state()`, and `warm_up()`. Deprecation without a removal plan is noise.

- [x] Decide: is `BasicReverb` being removed in the next version, or is it staying?
- [x] If removing: replace all matches with the `DelayReverb` arm and delete the variant
- [ ] If keeping: document the timeline in a comment and leave a note in the changelog

### 4.4 Reduce public API surface

Exposing internal types makes future refactoring a breaking change.

- [x] Make `RuntimeInstanceMeta`, `RuntimeInstancePlan`, `ActiveWindow`, and `ShuffleSource` in `container/prot.rs` `pub(crate)` or module-private
- [ ] Make `TrackBuffer` and `TrackBufferMap` in `audio/buffer.rs` `pub(crate)` (they are internal engine types)
- [ ] Give `EffectContext` private fields and a constructor that validates invariants (non-zero channels, non-zero sample rate)
- [ ] Make `PlaySettingsLegacy`, `PlaySettingsV1`, etc. `pub(crate)` — the versioned deserialization is an internal concern

### 4.5 Unify EOS detection between `track/single.rs` and `track/container.rs`

**Files:** `proteus-lib/src/track/single.rs`, `proteus-lib/src/track/container.rs`

Both implement decode-thread loops with `track_eos_ms` tracking and shutdown logic independently. A behavioral difference in EOS detection between the two paths could cause subtle alignment differences.

- [ ] Extract shared EOS timeout logic and shutdown checking into a common helper
- [ ] Verify the EOS semantics are identical across both paths with a test that exercises both

### 4.6 Fix `instance_needs_data()` naming

**File:** `proteus-lib/src/playback/engine/mix/buffer_mixer/helpers.rs` lines 143–149

The function always returns `true` but its name implies conditional logic.

- [ ] Either rename to reflect the unconditional intent (e.g., `always_needs_data` or `decoder_demand_enabled`) or remove the function and replace call sites with a literal `true` with a comment

### 4.7 Extract shared DSP utility functions

Gain clamping on NaN, channel-count `.max(1)` guards, and similar patterns are duplicated across multiple effect implementations and the mixing engine.

- [ ] Create a `dsp/utils.rs` module (or extend the existing one) with shared helpers for NaN-safe gain clamping and channel count sanitization
- [ ] Replace duplicated implementations with calls to the shared helpers

---

## Verification Checklist (Run After Each Phase)

```
cargo check -p proteus-lib
cargo clippy -p proteus-lib -- -D warnings
cargo test -p proteus-lib --lib
cargo check -p proteus-cli
cargo test -p proteus-cli
cargo run -p proteus-cli -- <test.prot>           # smoke test playback
cargo run -p proteus-cli --features debug -- <test.prot>  # verify no new warnings in debug output
```

---

## Summary Table

| Phase | Completed | Item | Severity | Files |
|---|---|---|---|---|
| 1 | Yes | Panics in `reduce_track_infos` | Critical | `container/info.rs` 471, 478, 488, 512 |
| 1 | Yes | File-open panic in `get_probe_result_from_string` | Critical | `container/info.rs` 71–73 |
| 1 | Yes | `catch_unwind` as error handling | Critical | `container/info.rs` 152–162 |
| 1 | No | Decoder selection bug (`tracks()[0]`) | Critical | `tools/tools.rs` 64–72 |
| 1 | Yes | Panic on no decodable audio track | Critical | `track/single.rs` 57–71 |
| 2 | No | Atomic ordering inconsistency | High | `runtime/thread.rs`, `settings.rs`, `worker/runner.rs` |
| 2 | No | Per-chunk `Vec` allocations in effect chain | High | `mix/effects.rs` 17–27 |
| 2 | Yes | `println!` in `Prot::new` | High | `container/prot.rs` 83 |
| 2 | No | Effects mutex held during DSP chain | High | `mix/runner/mod.rs` ~482 |
| 2 | No | `.lock().unwrap()` cascade failure risk | High | Throughout |
| 3 | No | 20ms/5ms polling in playback worker | Medium | `worker/runner.rs` 115, 510 |
| 3 | No | `Condvar::wait` without timeout | Medium | `buffer_mixer/backpressure.rs` 114 |
| 3 | No | Silent seek and decode failures | Medium | `track/single.rs` 92, 99–108 |
| 3 | No | Decode worker collapses errors to EOS | Medium | `decode/file_worker.rs`, `container_worker.rs` |
| 3 | No | Single-container always does full-file scan | Medium | `container/info.rs` 572 |
| 3 | No | O(n) `VecDeque::drain` in premix | Medium | `playback/engine/premix.rs` 52 |
| 3 | No | HashMap lookups in inner mixing loop | Medium | `mix/track_mix.rs` 82–95 |
| 3 | No | `SamplesBuffer` double-clone | Medium | `audio/samples.rs` 19–27 |
| 3 | No | Track weighting unimplemented (standalone) | Medium | `track/single.rs` 46 |
| 3 | No | Float-to-`usize` without sign guard | Medium | `buffer_mixer/helpers.rs` 129–132 |
| 4 | No | Remove all identified dead code | Low | Multiple |
| 4 | Yes | Split oversized files | Low | `prot.rs`, `runner.rs`, `buffer_mixer/mod.rs`, `info.rs` |
| 4 | Yes | Complete/remove `BasicReverb` deprecation | Low | `dsp/effects/mod.rs` |
| 4 | No | Reduce public API surface | Low | `prot.rs`, `audio/buffer.rs`, `dsp/effects/mod.rs`, `play_settings/mod.rs` |
| 4 | No | Unify EOS detection between decode paths | Low | `track/single.rs`, `track/container.rs` |
| 4 | No | Fix `instance_needs_data()` naming | Low | `buffer_mixer/helpers.rs` 143–149 |
| 4 | No | Extract shared DSP utilities | Low | Multiple effect files |
