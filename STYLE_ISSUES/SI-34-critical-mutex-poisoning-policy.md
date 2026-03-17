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

- [x] Critical runtime mutexes have an explicit, documented poison-handling policy
- [x] Recovery-capable mutexes no longer default to panic-on-poison
- [x] Invariant-only panic sites are intentionally documented as such

## Status

Closed.

## Resolution

Critical playback/runtime mutexes now acquire through centralized poison-policy helpers instead of
ad hoc `lock().unwrap_or_else(...)` sites.

- `Player`, `ThreadContext`, `LoopState`, `MixLoopState`, and `DecodeBackpressure` now declare
  each critical mutex as either `recoverable` or `invariant-only` behind helper accessors.
- `PlayerEngine` and `WorkerNotify` now also use the same centralized poison-policy helpers for
  engine buffer bookkeeping, container metadata access, and worker wake coordination.
- Recoverable runtime state now uses `into_inner()`-based recovery through
  `playback::mutex_policy::lock_recoverable` / `wait_recoverable`, covering sink/effects state,
  runtime telemetry, inline-update queues, backpressure bookkeeping, and related coordination
  mutexes.
- Invariant-only sites are now explicitly documented where state-machine or container coherence is
  required, and continue to panic intentionally through `lock_invariant`.
- Recovery behavior is covered by unit tests for the shared helper, player sink/effects locks, and
  decode backpressure recovery, including timed-condvar and worker-notify recovery paths.
