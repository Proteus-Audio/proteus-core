# SI-19: Premix Buffer Uses an O(n) Front-Drain Queue

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/playback/engine/premix.rs` | Replaced `VecDeque<f32>` with `Vec<f32>` + head/tail indices; added `pop_chunk_into` and `compact()` |
| `proteus-lib/src/playback/engine/mod.rs` | Declared `premix` as a `pub(crate)` module |
| `proteus-lib/src/playback/engine/mix/runner/state.rs` | Changed `pending_mix_samples` from `Vec<f32>` to `PremixBuffer` |
| `proteus-lib/src/playback/engine/mix/runner/loop_body.rs` | Replaced `drain(0..batch).collect()` and `extend_from_slice` with `pop_chunk` and `push_interleaved` |

---

## Current state

`PremixBuffer` stores samples in a `VecDeque<f32>` and removes output with:

```rust
self.samples.drain(0..take).collect()
```

This is the wrong shape for a steady stream that repeatedly appends at the back and consumes from
the front.

### Why this matters

- The premix queue is touched for every emitted DSP chunk
- O(n) front-drain work adds avoidable copy/shuffle overhead
- This buffer exists specifically to decouple mixing cadence from DSP cadence, so it should be as
  cheap and predictable as possible

### Recommended remediation

1. Replace the internal `VecDeque<f32>` with a true ring-buffer structure or explicit head/tail
   indices over a contiguous `Vec<f32>`
   The roadmap specifically points out that `dasp_ring_buffer` is already a dependency, so reuse of
   that crate is preferred over introducing another queue abstraction.
2. Provide APIs that copy directly into a caller-provided output slice where possible
3. If returning owned chunk vectors remains necessary, still avoid front-drain shifting by reading
   from the ring into a pre-sized output buffer
4. Size the buffer around the existing chunking policy so wrap-around is normal and inexpensive
5. Add micro-level tests for push/pop ordering, wrap-around, and partial final chunks

### Acceptance criteria

- [x] `PremixBuffer::pop_chunk` no longer relies on `VecDeque::drain(0..take)`
- [x] The premix buffer uses O(1) amortized front-consumption semantics
- [x] Ordering and partial-chunk behavior are covered by tests
- [x] Chunk pop semantics remain identical to the current buffer from the caller's perspective

## Status

Done.
