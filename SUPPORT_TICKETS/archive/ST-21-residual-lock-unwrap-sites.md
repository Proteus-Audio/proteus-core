# SI-21: Residual `lock().unwrap()` Sites Remain Outside the Main Library Sweep

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/diagnostics/reporter.rs` | Non-test code still uses raw `lock().unwrap()` in report callbacks and state mutation |
| `proteus-cli/src/logging.rs` | TUI log capture uses raw `lock().unwrap()` for shared buffer updates and snapshots |
| `proteus-lib/src/*` tests | Multiple test-only `lock().unwrap()` calls remain; these are lower risk but still inconsistent |

---

## Current state

`SI-09` cleaned up the bulk of production `proteus-lib` locking, but a repository sweep still finds
raw `lock().unwrap()` sites in remaining non-test code, especially CLI logging and diagnostics.

Representative cases:

- `proteus-cli/src/logging.rs`
- `proteus-lib/src/diagnostics/reporter.rs` tests and callback setup

### Why this matters

- The project now has two lock-handling styles, which makes future regressions likely
- CLI and diagnostics code are still user-facing and should not crash opaquely on poisoned locks
- Leaving residual exceptions weakens the benefit of the earlier cleanup

### Recommended remediation

1. Extend the `SI-09` policy to the rest of the workspace:
   - production library code
   - CLI runtime code
   - diagnostics support code
2. Replace remaining production `lock().unwrap()` sites with either:
   - `unwrap_or_else(|_| panic!("... lock poisoned ..."))` for invariant-only contexts, or
   - typed error propagation where the function already returns `Result`
3. Decide whether test code is exempt; if yes, document that exemption explicitly
4. Add a repo-wide lint/grep guard that excludes `#[cfg(test)]` if needed but rejects new
   production `lock().unwrap()` sites

### Acceptance criteria

- [x] No production `lock().unwrap()` sites remain in `proteus-lib/src` or `proteus-cli/src`
- [x] Any allowed test-only exceptions are documented explicitly
- [x] CI prevents reintroduction of production raw lock unwraps

### Test-only exemption

`lock().unwrap()` is permitted inside `#[cfg(test)]` modules. Test code runs in
a controlled environment where a poisoned lock indicates a test bug, not a
user-facing crash. The remaining test-only sites are in:

- `proteus-lib/src/track/buffer.rs`
- `proteus-lib/src/track/container.rs`
- `proteus-lib/src/track/single.rs`
- `proteus-lib/src/playback/player/controls.rs`
- `proteus-lib/src/playback/player/runtime/worker/timing.rs`
- `proteus-lib/src/playback/player/runtime/worker/runner.rs`
- `proteus-lib/src/playback/engine/mix/track_mix.rs`
- `proteus-lib/src/playback/engine/mix/runner/decode/mod.rs`
- `proteus-lib/src/diagnostics/reporter.rs`
- `proteus-cli/src/logging.rs`

## Status

Done.
