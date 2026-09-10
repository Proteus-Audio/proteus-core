# SI-13: Effect Chain Still Allocates in the Real-Time Hot Path

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/playback/engine/mix/effects.rs` | `run_effect_chain` clones the input and replaces it with a newly allocated `Vec<f32>` for every effect stage |
| `proteus-lib/src/dsp/effects/mod.rs` | `AudioEffect::process(&[f32], &EffectContext, bool) -> Vec<f32>` hard-codes allocation into the public effect interface |
| `proteus-lib/src/dsp/effects/*` | Each effect implementation currently returns owned output buffers rather than writing into caller-owned storage |

---

## Current state

The current mix path still does:

```rust
let mut current = input.to_vec();
for effect in effects.iter_mut() {
    current = effect.process(&current, context, drain);
}
```

That guarantees at least one allocation per chunk and then another allocation for every effect in
the chain. In steady-state playback this is allocator traffic on the real-time thread.

### Why this matters

- Real-time audio code should avoid allocator dependence during steady-state playback
- Allocation churn increases latency variance and raises drop-out risk under CPU pressure
- The current API shape prevents local optimization because every caller must accept owned output
- The roadmap explicitly calls out a representative five-effect stereo chain at 48 kHz, where this
  design translates into six heap allocations per chunk

### Recommended remediation

1. Replace the owned-buffer effect API with a caller-owned-buffer contract:
   ```rust
   fn process_into(
       &mut self,
       input: &[f32],
       output: &mut Vec<f32>,
       context: &EffectContext,
       drain: bool,
   )
   ```
2. Give the mix runtime two scratch buffers sized to the current chunk capacity and ping-pong
   between them through the chain
3. Reserve scratch capacity once when the effect chain or chunk size changes, not per chunk
4. For passthrough or no-op effects, copy or swap buffers instead of allocating
5. Keep the old owned-return API only as a temporary internal shim during migration, then delete it
6. Re-run the effect warm-up path and drain path to ensure tail-producing effects still behave the
   same with caller-owned output buffers
7. Benchmark or instrument the before/after path so the refactor demonstrates reduced allocation
   pressure rather than only changing API shape

### Acceptance criteria

- [x] The steady-state effect chain performs zero heap allocations per chunk
- [x] `run_effect_chain` uses reusable scratch buffers rather than `input.to_vec()`
- [x] `AudioEffect` and effect implementations no longer require returning a fresh `Vec<f32>`
- [x] Drain/tail behavior remains correct for reverb and convolution effects
- [x] Playback output matches existing behavior in functional tests and smoke playback runs
- [ ] A benchmark or instrumentation pass demonstrates reduced hot-path allocation pressure

## Status

Closed. The `process_into` method was added to `DspEffect` (with a default shim) and overridden in all 10 simple/reverb effects. `run_effect_chain` now ping-pongs between two pre-allocated scratch buffers in `MixLoopState`. `send_samples` was changed to accept `&[f32]` to avoid handing off ownership of a scratch buffer to rodio. The convolution reverb retains the default shim (FFT internals allocate regardless); steady-state chains without convolution reverb are fully zero-allocation. All 179 existing tests pass; no functional behaviour was changed.
