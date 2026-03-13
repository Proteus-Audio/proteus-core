# SI-34: Critical Runtime Mutexes Still Lack an Intentional Poison-Recovery Policy

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/playback/player/*` | Critical state mutexes now have descriptive panic messages but still default to panic-on-poison |
| `proteus-lib/src/playback/engine/*` | Effects, sink, and runtime coordination mutexes remain panic-on-poison rather than policy-driven |

---

## Current state

The broad `lock().unwrap()` cleanup is largely complete in non-test library code, but the roadmap's
stronger ask is still open: critical runtime mutexes do not yet have an explicit recovery or
poison-handling policy. They now fail more descriptively, but not more intentionally.

### Why this matters

- "Descriptive panic" and "poisoning strategy" are not the same thing
- Some mutexes may be valid invariant-only panic sites, while others could recover safely
- The current state leaves that distinction undocumented and unreviewed

### Recommended remediation

1. Classify critical mutexes by policy:
   - invariant-only: panic with descriptive context
   - recoverable: continue with `into_inner()` or return a typed error
2. Document that policy near each critical shared-state field or behind helper accessors
3. Centralize lock acquisition for critical mutexes so poison behavior is not duplicated at every
   call site
4. Add tests for any mutexes that choose recovery semantics

### Acceptance criteria

- [ ] Critical runtime mutexes have an explicit, documented poison-handling policy
- [ ] Recovery-capable mutexes no longer default to panic-on-poison
- [ ] Invariant-only panic sites are intentionally documented as such

## Status

Open.
