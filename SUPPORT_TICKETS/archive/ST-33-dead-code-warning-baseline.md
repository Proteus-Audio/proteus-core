# SI-33: Dead-Code Cleanup Is Partly Done but Still Lacks a Clean Warning Baseline

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/logging/mod.rs` | Internal logging helpers still need an explicit audit for necessity and warning cleanliness |
| `proteus-lib/src/*` | The roadmap's final dead-code acceptance criterion is a clean `cargo clippy` sweep, which has not been revalidated here |

---

## Current state

The remaining internal logging helpers are still live behind feature gates, and the crate's
dead-code baseline has now been revalidated.

### Why this matters

- Dead-code cleanup is only durable if the warning baseline is actually clean
- Leaving the last verification step undone invites new dead code to accumulate unnoticed
- The internal logging module is feature-gated enough that stale code can linger quietly

### Remediation performed

1. Audited `proteus-lib/src/logging/mod.rs` and confirmed the helpers are still used by the
   `buffer-map` and `debug` diagnostics paths
2. Documented that intentional feature-gated usage directly in `proteus-lib/src/logging/mod.rs`
3. Ran `cargo clippy -p proteus-lib -- -D warnings` on March 17, 2026 and confirmed a clean lint baseline
4. Ran `cargo clippy -p proteus-lib --all-features -- -D warnings` on March 17, 2026 to verify the
   feature-gated logging paths are also warning-clean when enabled

### Acceptance criteria

- [x] Remaining internal logging helpers are either justified by feature use or removed
- [x] `cargo clippy -p proteus-lib -- -D warnings` is clean with respect to dead-code findings
- [x] Any intentional feature-gated exceptions are documented

## Status

Closed on March 17, 2026.
