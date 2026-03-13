# SI-14: Playback Thread Atomics Lack a Clear Cross-Thread Memory Contract

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/playback/player/runtime/thread.rs` | Writes `playback_thread_exists` with `SeqCst` and resets `buffering_done` during thread startup |
| `proteus-lib/src/playback/player/settings.rs` | Reads `playback_thread_exists` with `SeqCst` but still reads `buffering_done` with `Relaxed` |
| `proteus-lib/src/playback/player/lifecycle.rs` | Clears `buffering_done` with `Relaxed` during lifecycle transitions |
| `proteus-lib/src/playback/player/runtime/worker/timing.rs` | Stores `buffering_done = true` with `Relaxed` when producer buffering completes |

---

## Current state

The playback runtime uses atomics as cross-thread state signals, but the ordering story is mixed.
The most obvious remaining problem is `buffering_done`, which is written and read across threads
using `Ordering::Relaxed`.

### Why this matters

- Cross-thread readiness flags should have a documented happens-before contract
- `Relaxed` is only correct when visibility and ordering do not matter; here they do
- Mixed ordering choices make the code harder to audit and easier to regress later

### Recommended remediation

1. Document the intended ownership and visibility rules for each playback atomic:
   `playback_thread_exists`, `buffering_done`, `audio_heard`, `playback_id`, timing markers
2. Use `Release` on stores that publish a completed state transition and `Acquire` on readers that
   consume that transition
3. Keep `SeqCst` only where a true global ordering requirement exists; otherwise prefer a simpler
   per-flag acquire/release contract
4. Wrap direct loads/stores in helper methods so ordering is centralized instead of repeated at
   call sites
5. Add targeted tests for startup, shutdown, and buffering completion state transitions

Example direction:

```rust
buffering_done.store(true, Ordering::Release);
let done = buffering_done.load(Ordering::Acquire);
```

### Acceptance criteria

- [ ] Every cross-thread playback atomic has a documented ordering contract
- [ ] `buffering_done` no longer uses `Relaxed` for cross-thread publication/observation
- [ ] Ordering choices are centralized behind helper methods or tightly scoped wrappers
- [ ] Startup/shutdown/buffering tests cover the intended state transitions

## Status

Open.
