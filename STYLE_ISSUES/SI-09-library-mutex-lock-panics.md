# SI-09: Library Mutex Locking Still Panics Pervasively

## Files affected

This issue spans most runtime-heavy library modules. Representative hotspots:

| File | Approx. occurrences |
|---|---|
| `proteus-lib/src/playback/player/*` | many direct `lock().unwrap()` calls on shared runtime state |
| `proteus-lib/src/playback/engine/*` | many direct `lock().unwrap()` calls on engine state |
| `proteus-lib/src/track/*` | direct panicking lock acquisition in decode/buffer paths |
| `proteus-lib/src/diagnostics/reporter.rs` | direct panicking lock acquisition in reporter loop |
| `proteus-lib/src/dsp/effects/convolution_reverb/ir_loader.rs` | direct panicking cache lock acquisition |

---

## Current state

A repository sweep currently finds **185 `lock().unwrap()` occurrences** under
`proteus-lib/src`.

The style guide explicitly forbids `.unwrap()` on `Mutex::lock()`, requiring either typed error
propagation or a descriptive `unwrap_or_else` in non-`Result` contexts. The current pattern means
poisoned locks panic across the library, including playback and diagnostics surfaces that should be
embeddable.

### Specific style-guide violations

- `.unwrap()` on `Mutex::lock()` is used throughout library code
- Panics are reachable in `proteus-lib` outside documented invariant-only contexts
- Real-time and background-thread code mixes lock acquisition with panic-on-poison behavior

### Why this matters

Lock poisoning should not turn the library into an uncontrolled panic surface. In embedding
contexts, this makes the crate harder to recover from and harder to reason about under failure.
The pattern is also inconsistent: some code already uses `if let Ok(...)` or other non-panicking
paths.

### Recommended remediation

1. Introduce a small set of helper utilities for lock acquisition:
   - typed `Result`-returning helpers for fallible API paths
   - `unwrap_or_else` helpers with module-specific panic messages for invariant-only internal paths
2. Prioritize public API and background-thread entry points first:
   - `playback/player/*`
   - `playback/engine/*`
   - `track/*`
   - `diagnostics/reporter.rs`
3. Audit hot-path locking after conversion so error handling does not add avoidable allocations or
   extra lock churn
4. Add a grep- or clippy-backed CI check to prevent reintroduction once the count is driven to
   zero

### Acceptance criteria

- [ ] No raw `lock().unwrap()` calls remain in `proteus-lib/src`
- [ ] Public fallible APIs propagate lock failures through typed errors where appropriate
- [ ] Invariant-only lock acquisitions use descriptive `unwrap_or_else` messages
- [ ] Hot-path changes are validated to avoid behavioral regressions in playback timing

## Status

Open.
