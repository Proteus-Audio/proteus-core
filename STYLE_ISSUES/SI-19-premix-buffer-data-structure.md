# SI-19: Premix Buffer Uses an O(n) Front-Drain Queue

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/playback/engine/premix.rs` | `PremixBuffer::pop_chunk` drains from the front of a `VecDeque` and collects into a new `Vec` on every chunk |

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
2. Provide APIs that copy directly into a caller-provided output slice where possible
3. If returning owned chunk vectors remains necessary, still avoid front-drain shifting by reading
   from the ring into a pre-sized output buffer
4. Size the buffer around the existing chunking policy so wrap-around is normal and inexpensive
5. Add micro-level tests for push/pop ordering, wrap-around, and partial final chunks

### Acceptance criteria

- [ ] `PremixBuffer::pop_chunk` no longer relies on `VecDeque::drain(0..take)`
- [ ] The premix buffer uses O(1) amortized front-consumption semantics
- [ ] Ordering and partial-chunk behavior are covered by tests

## Status

Open.
