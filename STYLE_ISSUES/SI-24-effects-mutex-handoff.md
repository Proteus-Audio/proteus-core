# SI-24: Mix Runtime Still Holds the Shared Effects Mutex While Running DSP

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/playback/engine/mix/runner/effects_runtime.rs` | `process_effects` and `drain_effect_tail` run `run_effect_chain` while holding `state.effects` |
| `proteus-lib/src/playback/player/effects.rs` | Control-path APIs share the same effects mutex and can contend with DSP work |

---

## Current state

The mix thread still does:

```rust
run_effect_chain(
    &mut state.effects.lock().unwrap_or_else(...),
    samples,
    &state.effect_context,
    false,
)
```

That means the mutex protecting the effect chain is held across all DSP processing for the chunk.

### Why this matters

- Convolution or large effect chains can hold the lock for relatively long stretches
- Control paths such as inline effect updates or effect queries contend with the same mutex
- This creates a priority inversion risk between user/control work and real-time DSP work

### Recommended remediation

1. Move from "shared mutable chain behind one mutex" to one of these handoff patterns:
   - per-chunk clone/snapshot of the effect chain
   - double-buffered effect state with atomic handoff
   - command queue for updates plus real-time-owned effect state
2. Keep the real-time thread's steady-state processing outside the shared mutex
3. Reconcile inline effect transitions with the chosen handoff pattern so `set_effects_inline`
   still applies deterministically
4. Audit `drain_effect_tail` as well as normal chunk processing; both paths must avoid long mutex
   hold times

### Acceptance criteria

- [ ] The real-time mix thread no longer holds the shared effects mutex while executing the full DSP chain
- [ ] Inline effect updates still apply correctly with the new ownership/handoff model
- [ ] Drain/tail processing follows the same low-contention design

## Status

Open.
