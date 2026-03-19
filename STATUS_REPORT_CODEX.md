# proteus-lib Status Report

## Scope

This report is based on a code review of `proteus-lib/src`, plus validation with:

- `cargo check -p proteus-lib`
- `cargo test -p proteus-lib --lib`

Current state:

- `cargo check` passes.
- `cargo test` passes with `62` passing unit tests.
- The crate still emits a large number of warnings (unused imports, dead code, unused helper modules, and unused public re-exports).

Overall, `proteus-lib` is functional and actively maintained, but it has visible architectural drift: the public-facing module structure is clean, while several internal areas have accumulated stale code paths, panic-based error handling, and some real-time performance risks.

## High-Level Assessment

Condition: usable, but carrying moderate technical debt.

What is strong:

- The top-level crate structure is coherent. The domain split between `container`, `playback`, `dsp`, and `peaks` is easy to follow.
- A number of public APIs now have crate-level and item-level documentation, especially in `lib.rs`, `playback/player`, and `peaks`.
- The DSP/effects layer is fairly well covered by unit tests, and the test suite passes.
- The newer mixing path under `playback/engine/mix/runner` shows evidence of careful concurrency work, especially around decode-worker shutdown and buffering coordination.

What is weak:

- Several very large source files are doing too much and are becoming maintenance bottlenecks.
- Some library code still panics on invalid input or ordinary I/O failures instead of returning errors.
- There is a noticeable amount of dead or half-retired code still compiled into the crate.
- The real-time path still relies on `std::sync::Mutex` and polling/sleep loops in places where latency and scheduling jitter matter.

## File and Module Organization

### What is organized well

- `proteus-lib/src/lib.rs` is minimal and presents a clean public surface.
- `container`, `playback`, `dsp`, and `peaks` are logically separated.
- `playback/player` is split into `controls`, `effects`, `runtime`, and `settings`, which is a good direction.
- `playback/engine/mix` is structured into smaller support modules (`buffer_mixer`, `decoder_events`, `runner`, `types`, etc.), which is better than a single-file engine.

### Where organization is degrading

The biggest maintainability issue is file size and concentration of responsibilities:

- `proteus-lib/src/container/prot.rs`: 1741 lines
- `proteus-lib/src/playback/engine/mix/buffer_mixer/mod.rs`: 846 lines
- `proteus-lib/src/dsp/effects/multiband_eq.rs`: 778 lines
- `proteus-lib/src/playback/player/runtime/worker/runner.rs`: 749 lines
- `proteus-lib/src/dsp/effects/diffusion_reverb/mod.rs`: 710 lines
- `proteus-lib/src/peaks/format.rs`: 680 lines
- `proteus-lib/src/playback/engine/mix/runner/mod.rs`: 650 lines
- `proteus-lib/src/container/info.rs`: 614 lines

These files are not just long; several combine parsing, policy, runtime control, and utility logic in one place. In particular:

- `container/prot.rs` mixes container model, shuffle scheduling, runtime planning, track mix settings, combination counting, and tests.
- `container/info.rs` mixes metadata probing, fallback parsing, format-specific parsing, duration scanning, and sample format reduction.
- `playback/player/runtime/worker/runner.rs` owns transport orchestration, timing, sink management, buffering backpressure, and end-of-stream behavior in one file.

This does not mean the code is unusable, but it does increase review cost, change risk, and the chance of regressions when modifying behavior.

### Structural drift / dead code

`cargo check` and `cargo test` both report a substantial amount of dead code. The most notable cases:

- `ShuffleRuntimePlan` and `build_runtime_shuffle_plan` in `proteus-lib/src/container/prot.rs` are no longer used.
- `OutputStageArgs` and `produce_output_samples` in `proteus-lib/src/playback/engine/mix/output_stage.rs` are not used.
- `SourceSpawner` in `proteus-lib/src/playback/engine/mix/source_spawner.rs` is not used.
- Multiple buffer diagnostics helpers in `playback/engine/mix/buffer_mixer/helpers.rs` are not used.
- `track/mod.rs` publicly re-exports APIs from a private module, but those re-exports are not used by the crate and are not exposed from `lib.rs`.
- The internal `logging` module has multiple unused functions.

This is a signal that the crate has gone through a partial refactor and some old paths were left in place. The cost is not just warnings; it makes it harder to tell which code is authoritative.

## Strengths

### Documentation and public API direction

The crate-level docs in `proteus-lib/src/lib.rs` and the doc comments in `playback/player` and `peaks` are better than average for an internal-heavy audio library. The intent of the major APIs is generally understandable without reverse-engineering the whole crate.

### Test coverage in critical logic

The test suite is not comprehensive, but it is stronger than the repository notes suggest. The 62 passing tests cover:

- DSP effect behavior and serde handling
- Peak file range/format behavior
- Shuffle scheduling and track mix planning
- Buffer mixer fill/readiness behavior

That is a real positive. It reduces risk in some of the more algorithmic parts of the crate.

### Evidence of real-time awareness

The newer mix-thread code includes some good practices:

- bounded channels for backpressure
- explicit decode-worker teardown to avoid shutdown deadlocks
- startup gating before playback begins
- effect warm-up ahead of live processing

This indicates the core playback path has been actively improved, even if some rough edges remain.

## Concrete Issues

### 1. Decoder selection bug in `tools::get_decoder`

File: `proteus-lib/src/tools/tools.rs`

Relevant lines:

- `get_reader` verifies that some supported audio track exists at lines 54-59.
- `get_decoder` still unconditionally builds the decoder from `format.tracks()[0]` at lines 64-72.

Why this is a problem:

- If track 0 is not the first decodable audio track (for example, a null/unsupported track appears before the real audio track), `get_reader` succeeds but `get_decoder` can still try to construct a decoder for the wrong track and panic with `unsupported codec`.
- This is a real correctness bug, not just a style issue.

Impact:

- Certain valid files can fail to open or panic depending on track ordering.

Severity: high

### 2. Library panics on normal file-open failures during probing

File: `proteus-lib/src/container/info.rs`

Relevant lines:

- `get_probe_result_from_string` opens the file with `File::open(path).expect("failed to open media file")` at lines 71-73.

Why this is a problem:

- This function returns `Result<ProbeResult, Error>`, so callers reasonably expect a recoverable error.
- Instead, a missing file, permission issue, or transient I/O failure will panic the library.
- That is especially problematic in a library crate expected to be embedded in other applications.

Impact:

- Host applications can crash on ordinary user input or filesystem issues.

Severity: high

### 3. Mixed-format input causes panic instead of validation error

File: `proteus-lib/src/container/info.rs`

Relevant lines:

- `reduce_track_infos` panics on mismatched sample rates at line 471.
- It panics on mismatched channel counts at lines 478-481.
- It panics on mismatched bit depth at line 488.
- It then calls `info.unwrap()` at line 512.

Why this is a problem:

- `Info::new_from_file_paths` feeds this path for user-supplied standalone files.
- A mixed input set (different sample rates, channel counts, or bit depths) is a validation failure, not an invariant violation.
- The current behavior crashes instead of returning an actionable error.

Impact:

- Multi-file playback setup is fragile and can take down the caller on invalid input combinations.

Severity: high

### 4. `Prot::new` writes directly to stdout from library code

File: `proteus-lib/src/container/prot.rs`

Relevant lines:

- `println!("Info: {:?}", info);` at line 83.

Why this is a problem:

- Library code should not print to stdout as part of normal object construction.
- It bypasses the crate’s own `log` usage and can pollute CLI output or GUI parent process logs.

Impact:

- Unexpected console noise and harder integration for downstream consumers.

Severity: medium

### 5. Real-time DSP path holds a mutex across full effect-chain processing

File: `proteus-lib/src/playback/engine/mix/runner/mod.rs`

Relevant lines:

- Effect resets lock `effects` at lines 303-307.
- Inline updates lock and replace the shared effect vector at lines 330-345.
- The steady-state DSP path locks `effects` and runs the full chain while holding the mutex at lines 482-483.

Why this is a problem:

- `run_effect_chain` can be expensive, especially with convolution reverb enabled.
- The same mutex is used by control/query code in `playback/player/effects.rs`.
- Holding a standard mutex across heavy DSP work creates a priority inversion risk: UI/control threads can block the audio path, or the audio path can block state changes unpredictably.

Impact:

- Increased risk of audible glitches or control lag under contention.

Severity: medium-high

### 6. Playback worker uses polling/sleep loops that add jitter and coarse wakeups

File: `proteus-lib/src/playback/player/runtime/worker/runner.rs`

Relevant lines:

- Main receive loop waits with `recv_timeout(Duration::from_millis(20))` at line 115.
- Sink backpressure waits using a loop with `thread::sleep(Duration::from_millis(5))` at line 510.

Why this is a problem:

- The worker is not purely event-driven; it relies on periodic wakeups.
- 20 ms polling in the main loop is relatively coarse for low-latency audio control.
- This may be acceptable for a CLI player, but it is not ideal for a reusable playback library where responsiveness matters.

Impact:

- Added transport latency, less precise pause/stop/seek response, and more scheduling jitter under load.

Severity: medium

### 7. File/container decode workers collapse errors into silent end-of-stream behavior

Files:

- `proteus-lib/src/playback/engine/mix/runner/decode/file_worker.rs`
- `proteus-lib/src/playback/engine/mix/runner/decode/container_worker.rs`

Relevant lines:

- `file_worker.rs` treats any `format.next_packet()` error as `break` at lines 71-74.
- `container_worker.rs` treats any `format.next_packet()` error as end-of-stream, sends `None`, and breaks at lines 85-90.
- `container_worker.rs` also logs broad decode failures and breaks on generic `Err(_)` at lines 169-170.

Why this is a problem:

- EOF, transient I/O failures, malformed packets, and unrelated format errors are all flattened into roughly the same control flow.
- The container worker additionally sends a global `None` signal, which the mixer interprets as finish-all.

Impact:

- Playback may terminate early on recoverable or diagnosable failures, and root-cause visibility is weak.

Severity: medium

### 8. Startup path for single-container `Info::new` is always full-file scan

File: `proteus-lib/src/container/info.rs`

Relevant lines:

- `Info::new` uses `get_durations_by_scan(&file_path)` directly at line 572.

Why this is a problem:

- Unlike `new_from_file_paths`, which uses metadata-first best-effort duration probing, single-container construction always scans all packets.
- For long containers, startup cost becomes O(file size) even when metadata could have been good enough.

Impact:

- Slower open times and avoidable startup I/O for large files.

Severity: medium

## Performance Notes

### Good performance choices

- The mix path uses bounded channels and explicit backpressure rather than unbounded accumulation.
- `BufferMixer` is clearly designed around interleaved sample buffers and startup gating, which is the right general shape for a real-time path.
- Convolution processing tries to align work to preferred batch sizes before running the effect chain.

### Main performance risks

- Mutex contention on the shared effect chain in the live DSP path.
- Polling/sleep loops in the playback worker instead of tighter signaling.
- Full-file duration scans during container initialization.
- Large, allocation-heavy control/data flow in the mix runner. Even in the newer path there are multiple `Vec` creations, drains, clones, and transition buffers in the hot loop.

None of these necessarily make the crate unusable, but they limit headroom and make it more likely to struggle under heavier DSP chains or slower machines.

## Potential Bug and Reliability Summary

Most likely real bugs:

- Wrong decoder chosen when the first track is not the first decodable track.
- Panics on file-open failures during probing.
- Panics on mismatched standalone input formats.
- Unwanted stdout output from a core constructor.

Most likely operational risks:

- Contention-induced glitches from holding the effects mutex during DSP work.
- Coarse worker polling leading to latency/jitter.
- Early termination when decode workers treat non-EOF errors as stream end.

## Testing and Build Health

### Verified

- `cargo check -p proteus-lib`: passes
- `cargo test -p proteus-lib --lib`: passes, 62/62 tests passing

### Still concerning

- The crate currently compiles with a large warning set.
- The warning volume makes it easier for real regressions to hide in the noise.
- The dead code footprint suggests ongoing refactors have not been fully cleaned up.

## Recommended Priorities

1. Replace panic-based error handling in `container/info.rs` and `tools/tools.rs` with proper `Result` returns.
2. Fix decoder selection to use the first supported track consistently.
3. Remove or gate the `println!` in `Prot::new`.
4. Reduce warning volume by deleting dead code or finishing the refactor that made it obsolete.
5. Move the live DSP path away from holding a shared mutex during full-chain processing.
6. Split `container/prot.rs`, `container/info.rs`, and `playback/player/runtime/worker/runner.rs` into smaller units by responsibility.

## Final Summary

`proteus-lib` is not in bad shape overall. The core direction is sound, the public structure is understandable, and the passing tests are a real strength. The main problems are concentrated in reliability and maintainability rather than complete design failure: too many panics in library code, too much dead code after refactors, and a few real-time/runtime choices that are serviceable but not ideal.

If the high-severity issues above are fixed and the stale code is trimmed back, the crate would move from “functional but carrying debt” to “solid and much easier to extend safely.”
