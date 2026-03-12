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

Lock-poisoning happens only when a thread panics while holding a lock, so the appropriate fix
differs by call site. Callers fall into two categories:

**Category A — invariant-only sites** (the vast majority): thread loops, hot-path decode and mix
code, and callbacks that cannot return `Result`. These must not return `Result` because callers
cannot handle the error. The fix is to add a descriptive message so a poisoned-lock panic is
diagnosable:

```rust
// Before
let guard = state.effects.lock().unwrap();

// After
let guard = state.effects.lock().unwrap_or_else(|_| {
    panic!("effects lock poisoned — a thread panicked while holding it")
});
```

**Category B — fallible public API paths** (a smaller minority): public `Player` methods and
engine entry points that already return `Result` or could do so without major API changes. These
can propagate the poisoned-lock error through a typed error variant instead of panicking:

```rust
pub fn get_buffer_settings(&self) -> Result<PlaybackBufferSettings, PlayerError> {
    self.buffer_settings
        .lock()
        .map(|g| g.clone())
        .map_err(|_| PlayerError::LockPoisoned)
}
```

**Recommended approach:**

1. Audit all 185 sites and classify each as Category A or B
2. For Category A (most sites): replace `.unwrap()` with `.unwrap_or_else(|_| panic!("... lock poisoned"))` — this is a mechanical, low-risk change
3. For Category B sites: add a `LockPoisoned` (or equivalent) error variant to the relevant error
   enum and propagate via `map_err`
4. Prioritize by module: `playback/player/*` → `playback/engine/*` → `track/*` → `diagnostics/`
5. Add a grep- or clippy-backed CI check to prevent reintroduction of bare `.unwrap()` on lock results

### Acceptance criteria

- [ ] No bare `lock().unwrap()` calls remain in `proteus-lib/src` (outside `#[cfg(test)]`)
- [ ] Category A sites use `unwrap_or_else(|_| panic!("... lock poisoned"))` with a descriptive message
- [ ] Category B sites propagate lock failures through typed error variants
- [ ] Hot-path changes are validated to avoid behavioral regressions in playback timing

## Status

Open.
