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

### Acceptance criteria

- [ ] The steady-state effect chain performs zero heap allocations per chunk
- [ ] `run_effect_chain` uses reusable scratch buffers rather than `input.to_vec()`
- [ ] `AudioEffect` and effect implementations no longer require returning a fresh `Vec<f32>`
- [ ] Drain/tail behavior remains correct for reverb and convolution effects
- [ ] Playback output matches existing behavior in functional tests and smoke playback runs

## Status

Open.
