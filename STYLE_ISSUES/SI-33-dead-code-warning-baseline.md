# SI-33: Dead-Code Cleanup Is Partly Done but Still Lacks a Clean Warning Baseline

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/logging/mod.rs` | Internal logging helpers still need an explicit audit for necessity and warning cleanliness |
| `proteus-lib/src/*` | The roadmap's final dead-code acceptance criterion is a clean `cargo clippy` sweep, which has not been revalidated here |

---

## Current state

Most of the roadmap's named dead-code removals appear to be completed, but the issue is not fully
closed because the warning baseline has not been re-established and the internal logging helpers are
still called out by the roadmap as needing review.

### Why this matters

- Dead-code cleanup is only durable if the warning baseline is actually clean
- Leaving the last verification step undone invites new dead code to accumulate unnoticed
- The internal logging module is feature-gated enough that stale code can linger quietly

### Recommended remediation

1. Audit `proteus-lib/src/logging/mod.rs` for feature-gated helpers that are no longer needed
2. Run `cargo clippy -p proteus-lib -- -D warnings` and treat dead-code warnings as actionable
3. Resolve any remaining dead-code warnings or document intentional feature-gated exceptions
4. Consider adding a CI guard so the dead-code baseline does not regress again

### Acceptance criteria

- [ ] Remaining internal logging helpers are either justified by feature use or removed
- [ ] `cargo clippy -p proteus-lib -- -D warnings` is clean with respect to dead-code findings
- [ ] Any intentional feature-gated exceptions are documented

## Status

Open.
