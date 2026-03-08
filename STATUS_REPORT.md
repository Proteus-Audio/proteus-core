# proteus-lib Status Report

Approximately 17,400 lines of Rust across 76 source files. The library has strong architectural bones — particularly the playback alignment model, convolution reverb caching, and effect-chain design — but several issues ranging from production panics to real-time allocation problems need attention.

---

## Module Organization

**Overall structure is clean.** The separation of `container/`, `playback/`, `dsp/`, `track/`, `audio/`, `diagnostics/`, and `peaks/` follows a coherent responsibility model, and the `track` module being crate-private is a correct design call. `logging` as `pub(crate)` is also appropriate.

**Issues:**

### `playback/player/runtime/worker/runner.rs` — 749 lines, multiple concerns
This is the largest file in the library and contains the playback worker loop, sink management, timing tracking, start-gate logic, and playback metrics aggregation all in one place. Each of those is a separable concern and the current size makes it difficult to test or change any single behavior in isolation.

### `track/single.rs` and `track/container.rs` share similar decode loop structure
Both implement decode-thread loops with buffering and EOS detection but do so independently, with slightly different patterns. The shared logic (buffer pushes, `track_eos_ms` tracking, shutdown checks) is duplicated rather than composed from common helpers. A subtle behavioral difference in EOS detection between the two could cause desync between single-file and container playback — this is worth verifying.

### Commented-out code in `container/info.rs` lines 211–222
A large `PartialEq` implementation is commented out with no explanation. Should be removed or documented.

### `instance_needs_data()` in `playback/engine/mix/buffer_mixer/helpers.rs` lines 143–149
The function always returns `true` and carries a comment explaining this is intentional for alignment semantics. The name implies conditional logic but it is unconditional. Either rename to communicate the intent or remove the function and inline a literal `true`.

---

## Public API Surface

**Issues:**

### Internal mixing types exposed through `container/prot.rs`
`RuntimeInstanceMeta`, `RuntimeInstancePlan`, `ActiveWindow`, and `ShuffleSource` are all `pub`. These are scheduling implementation details used by the mix engine. Exposing them couples external callers to the internal shuffling model and makes future refactoring harder.

### `TrackBuffer` / `TrackBufferMap` in `audio/buffer.rs` lines 10 and 12
These are `pub` type aliases over `Arc<Mutex<Bounded<Vec<f32>>>>`, exposing the specific ring-buffer crate (`dasp_ring_buffer`) as part of the public interface. Any change to the backing data structure becomes a breaking change.

### `EffectContext` in `dsp/effects/mod.rs` lines 42–50
`EffectContext` has all public fields and derives `Clone`. This means any caller can construct an arbitrary `EffectContext` with inconsistent values (e.g., `channels: 0`, mismatched `sample_rate`). It should have private fields and a controlled constructor.

### Play settings versioning leaked through `container/play_settings/mod.rs`
Multiple versioned structs (`PlaySettingsLegacy`, `PlaySettingsV1`, etc.) are public. The deserialization versioning strategy is an internal detail that need not be part of the public interface.

---

## Performance

### Allocation on every effect pass — real-time concern
`playback/engine/mix/effects.rs` lines 17–27:

```rust
let mut current = input.to_vec();  // allocation
for effect in effects.iter_mut() {
    current = effect.process(&current, context, drain);  // each call returns a new Vec
}
```

Every effect in the chain allocates a fresh `Vec<f32>`. For stereo 48 kHz audio at 30 ms chunks this is approximately 2,880 samples per chunk; with five effects in the chain that is six allocations per chunk, all from the allocator, all in the hot mixing path. Real-time audio must avoid the allocator in steady-state. The fix is to pre-allocate two scratch buffers and ping-pong between them, or to use in-place processing where possible.

Each `AudioEffect::process()` implementation also returns a freshly-allocated `Vec<f32>`. This is the root interface issue — the signature should take a mutable output slice rather than returning an owned buffer.

### `SamplesBuffer` double-clone in `audio/samples.rs` lines 19–27
`clone_samples_buffer()` collects an iterator into a `Vec<f32>`, then immediately clones the vector again to construct a `SamplesBuffer`. One of these copies is unnecessary.

### `VecDeque::drain(0..n)` in `playback/engine/premix.rs` line 52
`drain` on a `VecDeque` from the front is O(n) because it must shift elements. For the premix buffer this happens on every chunk. A ring buffer with read/write indices would be O(1) and this is exactly the access pattern `dasp_ring_buffer` is designed for — it is already a dependency.

### HashMap lookups in the tight mixing loop — `playback/engine/mix/track_mix.rs` lines 82–95
For every mix chunk, per-track weights and channel gain arrays are looked up by key from a snapshot `HashMap`. Since the set of active tracks is stable across a chunk, these could be snapshotted into parallel `Vec`s indexed by position at the start of the loop rather than resolved per-iteration.

### `std::panic::catch_unwind()` in `container/info.rs` lines 152–162
`catch_unwind` is used as an error handling mechanism in `get_durations_best_effort()`. Crossing the unwind boundary is expensive (stack unwinding, registering/deregistering the handler), and it signals that the underlying code is panicking on expected error conditions rather than returning `Result`. The panics should become errors.

### `Condvar::wait()` without timeout in `playback/engine/mix/buffer_mixer/backpressure.rs` line 114
```rust
guard = self.cv.wait(guard).unwrap();
```
If the thread that is supposed to notify this condvar panics or exits without notifying, this wait will block forever. A bounded `wait_timeout` with a shutdown check guards against this.

---

## Potential Bugs

### CRITICAL: Panics on valid-but-mismatched media input — `container/info.rs` lines 471, 478, 488

```rust
panic!("Sample rates do not match");
panic!("Channel layouts do not match {} != {}", ...);
panic!("Bits per sample do not match");
```

These are in `reduce_track_infos()`, which is called when combining metadata from multiple tracks in a container. If a user provides a container with tracks at different sample rates (entirely legal in MKA), the library crashes the process. These should return an error to the caller.

### HIGH: Panic on container file with no valid audio codec — `track/single.rs` lines 57–71

```rust
.expect("no track found")
```

Panics if the container probe succeeds but contains no decodable audio track. Malformed or non-audio files will crash. Should return an error through the decode-thread channel.

### HIGH: `.expect()` on file open — `container/info.rs` line 73

```rust
File::open(path).expect("failed to open media file")
```

Panics if the file is deleted or permission is denied between when the path was validated and when it is probed. Should return a `Result`.

### HIGH: Atomic ordering inconsistency on `playback_thread_exists`

`playback/player/runtime/thread.rs` line 41 writes `playback_thread_exists` with `Ordering::SeqCst`, but at least one read site (`player/settings.rs` line ~119) uses `Ordering::Relaxed`. On architectures with weaker memory models (ARM), a relaxed read after a SeqCst write may observe a stale value. All accesses to a given atomic should use a consistent, documented ordering. Since this flag is used to gate thread lifecycle decisions, the inconsistency is worth fixing even if it is harmless in practice today.

### HIGH: `Ordering::Relaxed` for `buffering_done` cross-thread signal

`playback/player/runtime/worker/runner.rs` line 644 stores `buffering_done = true` with `Ordering::Relaxed`, and `player/mod.rs` line 412 clears it with `Ordering::Relaxed`. A relaxed store on one thread is not guaranteed to be visible to another thread in a timely or ordered way without a synchronizing release/acquire pair. At minimum the store should be `Release` and the load `Acquire`.

### MEDIUM: Silent seek failure — `track/single.rs` line 92

```rust
if seek_success.is_err() { return; }
```

If a seek fails, the decode thread exits silently. No error is logged, no signal is sent up. The track will silently produce no audio and may be interpreted as finished by the mixer.

### MEDIUM: Decode errors discarded — `track/single.rs` lines 99–108

The return value of decode operations is bound to `_result` and dropped. Errors during decoding are silently swallowed and the loop continues, potentially producing incomplete audio without any diagnostic.

### MEDIUM: Float-to-usize cast without sign guard — `playback/engine/mix/buffer_mixer/helpers.rs` lines 129–132

```rust
let start_frame = (((overlap_start - packet_start) * sample_rate as f64).floor() as usize)
```

If `overlap_start - packet_start` is unexpectedly negative (e.g., due to floating-point rounding), the floor will be a small negative float, and casting to `usize` on Rust produces `0` on some platforms (saturating) and a very large number on others (the original C behavior). The clamping upstream should prevent this, but an explicit `.max(0.0)` before the cast would make the guarantee explicit and safe on all platforms.

### MEDIUM: Track weighting unimplemented for standalone file playback — `track/single.rs` line 46

```rust
// TODO: Apply `_track_weights` to scale per-track samples when weighting single-track buffers.
```

The `track_weights` parameter is accepted but ignored when playing standalone audio files. Per-track volume control is silently a no-op in this mode.

### LOW: ~50+ `.lock().unwrap()` calls throughout the codebase

If any thread panics while holding a mutex, the mutex is poisoned and every subsequent `.lock().unwrap()` on that mutex will also panic. This is a cascade failure mode. Using `.lock().expect("lock poisoned: <context>")` at minimum makes diagnostics easier; for production robustness, poisoning should be handled explicitly.

---

## Code Quality

### Deprecated variant still matched in `dsp/effects/mod.rs`

The `AudioEffect::BasicReverb` variant is marked `#[deprecated]` but is still matched in `process()` (line 95), `reset_state()` (line 114), and `warm_up()` (line 134) with `#[allow(deprecated)]` on each arm. This suggests the deprecation is not being driven toward removal. The variant should either be removed outright or the timeline for removal should be documented.

### `add_samples_to_buffer_map_nonblocking()` in `track/buffer.rs` lines 61–85

This function is annotated `#[allow(dead_code)]`. If it is kept as a planned future API, that should be documented. If it is genuinely unused, it should be removed.

### Gain clamping duplicated across multiple files

The pattern of clamping gain to `0.0` on `NaN` and the channel-count `.max(1)` guard appear independently in multiple effect implementations and in the mixing engine. These belong in a shared DSP utility, both to reduce duplication and to ensure consistent behavior.

---

## Things Done Well

### Virtual zero-fill in `playback/engine/mix/buffer_mixer/aligned_buffer.rs`
The aligned buffer tracks zero-fill regions as metadata segments rather than materializing large `Vec<f32>` zero buffers. This cleanly implements the alignment model described in `.guides/NOTES.md` without the memory cost.

### Convolution reverb caching — `dsp/effects/convolution_reverb/mod.rs`
```rust
static IMPULSE_RESPONSE_CACHE: OnceLock<Mutex<ImpulseResponseCacheMap>>;
static REVERB_KERNEL_CACHE: OnceLock<Mutex<ReverbKernelCacheMap>>;
```
IR loading and FFT kernel computation are cached globally behind `OnceLock`. Computing convolution kernels is expensive; this prevents redundant work when multiple instances share the same impulse response.

### Diffusion reverb per-channel decorrelated lanes
The diffusion reverb uses independent delay networks per channel rather than a shared interleaved network. This avoids the metallic cross-channel ringing that the shared approach produces. The `.guides/NOTES.md` constraint against reverting this is well-founded and the implementation respects it.

### Play settings versioning — `container/play_settings/mod.rs`
Multiple schema versions deserialize cleanly without crashing on unknown keys. Unknown versions fall through to a raw JSON capture rather than hard failing. This is a well-designed extension point for the settings format.

### Backpressure coordination — `playback/engine/mix/buffer_mixer/backpressure.rs`
The decode throttling correctly prevents buffer overflow in decoder threads while maintaining alignment across sources. The startup-phase priority scheme (favoring lagging sources during initial buffering) is a clean solution to the cold-start alignment problem.

### Numeric stability in DSP effects
Effects consistently use `clamp()`, `max()`, and `saturating_*()` operations. Biquad coefficient computation validates with `is_finite()` checks before applying. This prevents NaN/Inf propagation into the audio stream.

### Effect chain design — `dsp/effects/mod.rs`
The `AudioEffect` enum with a uniform `process(&[f32], &EffectContext, drain: bool) -> Vec<f32>` interface is clean and easy to extend. The `drain: bool` flag is an elegant way to handle tail-producing effects without a separate flush API. The `warm_up()` hook for lazy-initialization of expensive state (IR loading) is a good pattern.

### Play settings serde round-trip test coverage — `dsp/effects/mod.rs` tests
The test suite for `AudioEffect` covers both round-trip serialization and aliased/legacy JSON key names. This kind of regression coverage on serialized formats is exactly right given the versioned settings model.

---

## Summary Table

| Severity | Issue |
|---|---|
| Critical | Panics on mismatched track sample rates / channel layouts / bit depth (`container/info.rs` 471, 478, 488) |
| Critical | `catch_unwind` used as error handling for expected error conditions |
| High | Panic if container has no decodable audio track (`track/single.rs`) |
| High | Panic on file permission error (`container/info.rs` line 73) |
| High | Per-chunk Vec allocations in the effect chain hot path |
| High | Atomic ordering inconsistency on `playback_thread_exists` and `buffering_done` |
| Medium | Silent seek failure with no log or signal |
| Medium | Decode errors silently discarded in decode loop |
| Medium | Track weighting unimplemented for standalone file mode (TODO) |
| Medium | `Condvar::wait()` without timeout in backpressure |
| Medium | Float-to-usize cast without explicit sign guard |
| Medium | O(n) `VecDeque::drain` in premix buffer |
| Medium | HashMap lookups inside tight mixing loop |
| Low | `runner.rs` 749 lines, multiple concerns |
| Low | ~50 `.lock().unwrap()` calls, no poisoning strategy |
| Low | Dead code (`add_samples_to_buffer_map_nonblocking`) |
| Low | Deprecated `BasicReverb` variant still actively matched |
| Low | Internal mixing types (`RuntimeInstancePlan` etc.) exposed publicly |
| Low | `EffectContext` with fully public fields, no invariant enforcement |
| Good | Virtual zero-fill aligned buffer |
| Good | Convolution IR and kernel caching |
| Good | Per-channel diffusion reverb decorrelation |
| Good | Versioned play settings with graceful fallback |
| Good | Backpressure / decode throttle design |
| Good | Numeric stability guards in DSP code |
| Good | Effect chain interface with `drain` flag and `warm_up` |
