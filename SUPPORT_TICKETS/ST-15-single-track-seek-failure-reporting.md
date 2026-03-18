# SI-15: Single-Track Decode Seek Failures Still Collapse to Silent Termination

## Files affected

| File                              | Notes                                                                                                     |
| --------------------------------- | --------------------------------------------------------------------------------------------------------- |
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

### Additional roadmap context

The roadmap groups this with decode-loop observability more broadly:

- seek failure should log path and error
- decode failures should be surfaced distinctly from normal EOF
- repeated decode failures should have an intentional policy rather than default loop behavior

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
7. Decide whether repeated decode errors for a track should terminate that track after a threshold
   instead of warning indefinitely and continuing forever

### Acceptance criteria

- [x] Seek failure produces a structured diagnostic, not silent early return
- [x] Callers can distinguish decode failure from normal end-of-stream
- [x] Buffer waiters are still notified on failure paths
- [x] A regression test covers the failed-seek startup path
- [x] The policy for repeated decode failures is intentional and documented

## Resolution

`buffer_track` now returns `JoinHandle<TrackDecodeOutcome>` with three variants:
`Completed`, `Aborted`, and `Failed(String)`. Seek failures log the file path,
start time, and track id at `warn` level, notify condvar waiters, and return
`Failed`. A `finish()` helper ensures all exit paths mark finished and notify.

Per-packet `DecodeError` variants are logged at `warn` and the loop continues
(documented as intentional — transient codec errors are recoverable). Non-recoverable
errors break the loop and produce `Failed`.

Regression test `buffer_track_notifies_condvar_on_failure` covers the failure path.

## Status

Closed.
