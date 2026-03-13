# SI-15: Single-Track Decode Seek Failures Still Collapse to Silent Termination

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/track/single.rs` | `buffer_track` returns early when initial seek fails and does not emit a diagnostic or structured failure |

---

## Current state

The standalone decode worker now avoids panicking, but the initial seek path still does this:

```rust
if format.seek(...).is_err() {
    mark_track_as_finished(...);
    return;
}
```

That means a seek failure is indistinguishable from a normal exhausted track at the caller level.

### Why this matters

- Playback can fail silently with no clear explanation for the caller or logs
- Finished-track bookkeeping should mean "playback completed", not "setup failed"
- This hides malformed input, bad seek metadata, and decoder incompatibilities behind EOS behavior

### Recommended remediation

1. Log seek failures with the file path, target start time, and track id
2. Split "finished normally" from "failed during setup" in the worker outcome model
3. Thread a lightweight decode-worker status signal back to the caller:
   `Completed`, `Aborted`, `Failed(String)` is sufficient
4. Only mark the track finished automatically for true EOS or explicit abort, not for setup failure
5. Ensure any waiter/condvar notification still happens on failure so the runtime does not stall
6. Add a regression test that injects or simulates a seek failure and asserts a reported failure
   rather than a silent finish

### Acceptance criteria

- [ ] Seek failure produces a structured diagnostic, not silent early return
- [ ] Callers can distinguish decode failure from normal end-of-stream
- [ ] Buffer waiters are still notified on failure paths
- [ ] A regression test covers the failed-seek startup path

## Status

Open.
