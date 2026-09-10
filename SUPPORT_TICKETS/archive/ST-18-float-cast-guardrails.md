# SI-18: Packet Overlap Math Still Casts Floats to `usize` Without an Explicit Non-Negative Guard

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/playback/engine/mix/buffer_mixer/routing_helpers.rs` | `packet_overlap_samples` floors/ceils floating-point frame offsets and casts directly to `usize` |

---

## Current state

The overlap helper computes frame offsets from floating-point time math and then casts directly to
`usize`:

```rust
(((overlap_start - packet_start) * sample_rate as f64).floor() as usize)
```

The surrounding logic should normally keep this non-negative, but the invariant is implicit.

### Why this matters

- Safety assumptions around float rounding should be made explicit at the cast boundary
- Defensive clamping documents the intended invariant and prevents subtle future regressions
- This helper sits in alignment-sensitive mixing logic where bad indices are expensive to debug

### Recommended remediation

1. Clamp intermediate frame offsets explicitly before integer conversion:
   `(((value).max(0.0)).floor() as usize)` rather than relying on implicit non-negativity
2. Extract the conversion into a small helper so the invariant is named and reused
3. Add tests around boundary cases:
   - exact overlap start
   - tiny negative epsilon from rounding
   - overlap entirely before packet
4. Keep the existing upper clamp with `.min(frame_count)` after conversion

### Acceptance criteria

- [x] No float-to-`usize` cast in overlap math relies on implicit non-negativity
- [x] Boundary-condition tests cover small negative floating-point inputs
- [x] The helper remains behaviorally identical for valid inputs

## Status

Done.
