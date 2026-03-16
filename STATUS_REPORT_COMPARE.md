# Status Report Comparison

Comparing `STATUS_REPORT.md` (this session) against `STATUS_REPORT_CODEX.md` (Codex).

---

## What Both Reports Found

These findings were independently reached by both analyses.

### Bugs / Reliability
- **Panics in `container/info.rs` `reduce_track_infos`** on mismatched sample rates (line 471), channel layouts (line 478), and bit depth (line 488) — user-supplied input causes process crash.
- **Panic on file-open failure** in `container/info.rs` `get_probe_result_from_string` — a function that returns `Result` still calls `.expect()` internally.
- **Decode errors silently collapsed to end-of-stream** — in the file and container decode workers, I/O errors, format errors, and real EOF are all handled the same way, causing early playback termination without diagnostics.

### Performance
- **Mutex contention on the effects chain during live DSP** — the shared effects mutex is held while running the full effect chain, creating a priority inversion risk with control/query paths.
- **Allocation-heavy hot path in the mix runner** — multiple `Vec` creations, clones, drains, and transition buffers in the mixing loop.

### Organization
- **`playback/player/runtime/worker/runner.rs` at 749 lines** doing too much — transport orchestration, timing, sink management, buffering backpressure, and end-of-stream all in one file.
- **Dead/stale code from partial refactors** — both reports noted that the codebase shows evidence of incomplete cleanups, though with different specific examples.

---

## What the Codex Report Found That Mine Did Not

### Specific Concrete Bug I Missed
- **Decoder selection bug in `tools/tools.rs`** — `get_reader` correctly finds the first supported audio track, but `get_decoder` unconditionally uses `format.tracks()[0]` regardless of which track that is. If a null or unsupported track appears before the real audio track, `get_reader` succeeds and `get_decoder` then tries to build a decoder for the wrong track, panicking with "unsupported codec." Codex rated this **high** severity. I did not find this.

### Specific Issue I Missed
- **`Prot::new` calls `println!` to stdout** at `container/prot.rs` line 83 (`println!("Info: {:?}", info)`) as part of normal object construction. Library code printing to stdout bypasses the `log` crate, pollutes host application output, and cannot be suppressed by callers. I did not catch this.

### Startup I/O Issue I Missed
- **`Info::new` for single containers always does a full-file packet scan** (`get_durations_by_scan`) rather than attempting metadata-first probing. Unlike the multi-file path which tries best-effort metadata first, single-container construction is always O(file size) at open time. Codex rated this **medium**.

### File Sizes I Didn't Report
Codex enumerated all large files with line counts. I only called out `runner.rs`. The files Codex identified that I did not mention:
- `container/prot.rs` — **1,741 lines** (the largest file in the library)
- `playback/engine/mix/buffer_mixer/mod.rs` — 846 lines
- `dsp/effects/multiband_eq.rs` — 778 lines
- `dsp/effects/diffusion_reverb/mod.rs` — 710 lines
- `peaks/format.rs` — 680 lines
- `playback/engine/mix/runner/mod.rs` — 650 lines
- `container/info.rs` — 614 lines

### Specific Dead Code Items I Didn't Name
Codex called out specific identifiers by name:
- `ShuffleRuntimePlan` and `build_runtime_shuffle_plan` in `container/prot.rs` — no longer used
- `OutputStageArgs` and `produce_output_samples` in `playback/engine/mix/output_stage.rs` — not used
- `SourceSpawner` in `playback/engine/mix/source_spawner.rs` — not used
- `track/mod.rs` re-exports from a private module that are unused and not re-exported from `lib.rs`
- Multiple functions in the internal `logging` module that are unused

I noted the `add_samples_to_buffer_map_nonblocking` dead code case but missed these others.

### Worker Polling Patterns
Codex named specific sleep values in the worker loop: `recv_timeout(Duration::from_millis(20))` at line 115 and `thread::sleep(Duration::from_millis(5))` at line 510. I noted the absence of event-driven design and the condvar timeout issue but did not identify these specific polling values.

### Build Health Verification
Codex confirmed `cargo check` passes and `cargo test` shows 62/62 tests passing, and specifically noted that the crate compiles with a large warning volume that makes it easier for regressions to hide. My report did not verify build state or count tests.

---

## What My Report Found That the Codex Report Did Not

### Atomic Memory Ordering Inconsistency
- `playback_thread_exists` is written with `Ordering::SeqCst` in `runtime/thread.rs` line 41 but read with `Ordering::Relaxed` in `player/settings.rs`. Similarly, `buffering_done` uses `Ordering::Relaxed` for both its set and clear across different threads with no release/acquire pair. These atomics gate thread lifecycle decisions, so the inconsistency is a real correctness risk on non-x86 architectures.

### Track Weighting Unimplemented for Standalone File Mode
- `track/single.rs` line 46 has an open `TODO`: the `_track_weights` parameter is accepted but never applied. Per-track volume control is silently a no-op for standalone file playback. This is a behavior gap between container and standalone modes that neither the API nor any error communicates.

### O(n) `VecDeque::drain` in Premix Buffer
- `playback/engine/premix.rs` line 52 uses `drain(0..take)` on a `VecDeque`, which is O(n) because it must shift elements. This is on every emitted chunk. `dasp_ring_buffer` is already a dependency and is the appropriate data structure for this access pattern.

### HashMap Lookups Inside the Tight Mixing Loop
- `playback/engine/mix/track_mix.rs` lines 82–95 do per-iteration HashMap lookups for track weights and channel gains on every mix chunk. Since the active track set is stable within a chunk, these could be snapshotted into indexed `Vec`s at chunk start.

### `SamplesBuffer` Double-Clone
- `audio/samples.rs` lines 19–27: `clone_samples_buffer()` collects an iterator into a `Vec<f32>` and then immediately clones it again. One copy is unnecessary.

### `catch_unwind` Used as Error Handling
- `container/info.rs` lines 152–162: `std::panic::catch_unwind()` is used as a fallback in `get_durations_best_effort()`. This is extremely expensive (stack unwinding, panic handler registration) and signals that the underlying code panics on expected conditions rather than returning `Result`. My report flagged this separately from the panic issues themselves; Codex did not mention it.

### `Condvar::wait()` Without Timeout in Backpressure
- `playback/engine/mix/buffer_mixer/backpressure.rs` line 114 uses `self.cv.wait(guard)` with no timeout. If the thread that should send the notification exits abnormally, this blocks forever. Codex noted the polling/sleep patterns in the worker but did not catch this condvar issue.

### Float-to-`usize` Cast Without Explicit Sign Guard
- `playback/engine/mix/buffer_mixer/helpers.rs` lines 129–132: a `f64 * sample_rate` value is floored and cast directly to `usize`. If the operand is unexpectedly negative due to floating-point rounding, the cast behavior is platform-dependent (wraps to a very large number on some targets). An explicit `.max(0.0)` before the cast would make the safety guarantee explicit.

### Public API Leakage
My report documented several public API issues that Codex did not cover:
- `RuntimeInstanceMeta`, `RuntimeInstancePlan`, `ActiveWindow`, `ShuffleSource` are `pub` in `container/prot.rs` despite being internal scheduling concepts.
- `TrackBuffer` and `TrackBufferMap` in `audio/buffer.rs` are `pub` type aliases that expose the `dasp_ring_buffer` crate as part of the public interface — any backing change is a breaking change.
- `EffectContext` has fully public fields with no invariant enforcement, allowing arbitrary invalid construction.
- Multiple `PlaySettings*` versioned structs are public when they are an internal deserialization detail.

### Duplicate Track EOS Detection Between `single.rs` and `container.rs`
Both files implement the decode-thread loop independently with slightly different patterns for EOS detection and shutdown. A behavioral divergence between these two paths could cause subtle timing or alignment differences between single-file and container playback modes.

### `instance_needs_data()` Misleading Function Name
`playback/engine/mix/buffer_mixer/helpers.rs` lines 143–149: this function always returns `true` and carries a comment explaining it is intentional for alignment semantics. The name implies a conditional check. Either the name should communicate the unconditional intent or the function should be removed.

### Gain Clamping Logic Duplicated Across Files
The pattern of clamping gain to `0.0` on NaN and the channel-count `.max(1)` guard appear independently in multiple effect implementations and in the mixing engine rather than being shared utilities.

### Deprecated `BasicReverb` Still Actively Matched
`dsp/effects/mod.rs` matches on the deprecated `BasicReverb` variant with `#[allow(deprecated)]` in `process()`, `reset_state()`, and `warm_up()`. The deprecation is not being driven toward removal.

### Positive Findings My Report Made That Codex Did Not
Codex noted good practices in the mix-thread code (bounded channels, shutdown coordination, startup gating, effect warm-up) but did not document the following strengths that my report called out specifically:
- **Virtual zero-fill in `aligned_buffer.rs`** — zero-fill regions tracked as metadata segments, avoiding materialized large zero buffers.
- **Convolution IR and kernel caching** via `OnceLock<Mutex<...>>` globals — expensive FFT kernel computation is not repeated across instances sharing the same impulse response.
- **Per-channel decorrelated lanes in diffusion reverb** — correctly avoids the metallic ringing from a shared interleaved delay network.
- **Numeric stability guards in DSP effects** — consistent use of `clamp()`, `is_finite()`, and `saturating_*()` throughout effect processing.
- **Effect chain `drain` flag and `warm_up` hook** — clean interface for tail-producing effects and lazy initialization.
- **Play settings serde round-trip and alias test coverage** — regression tests for both serialization round-trips and legacy JSON key names.

---

## Summary

| Area | Only in Codex | In Both | Only in Mine |
|---|---|---|---|
| Bugs | Decoder selection bug (`tools.rs`), stdout in `Prot::new`, startup full-file scan | `reduce_track_infos` panics, file-open panic, silent decode errors → EOS | Atomic ordering inconsistency, float-to-usize cast, `catch_unwind` as error handling |
| Performance | Polling/sleep values (20ms, 5ms) | Mutex on effects during DSP, allocation-heavy hot path | `VecDeque::drain` O(n), HashMap in mix loop, `SamplesBuffer` double-clone, condvar without timeout |
| Organization | 7 additional large files with line counts, 5 specific dead code identifiers | `runner.rs` oversized, stale code from partial refactors | `instance_needs_data()` misleading name, `single.rs`/`container.rs` duplication |
| Public API | — | — | 4 specific public API leakage issues |
| Positives | Build health (62 tests, `cargo check` clean), doc quality | Real-time awareness in mix path | 6 specific positive findings (IR cache, zero-fill, decorrelation, etc.) |
