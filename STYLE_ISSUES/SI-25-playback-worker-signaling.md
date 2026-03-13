# SI-25: Playback Worker Still Uses Polling and Sleep-Based Wakeups

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/playback/player/runtime/worker/runner.rs` | Main worker loop still uses `recv_timeout(Duration::from_millis(20))` |
| `proteus-lib/src/playback/player/runtime/worker/sink.rs` | Sink backpressure still sleeps in 5 ms increments |

---

## Current state

The runtime still depends on coarse periodic wakeups:

- 20 ms `recv_timeout` for worker-control polling
- 5 ms `thread::sleep` loops for sink backpressure

### Why this matters

- Polling adds transport latency and scheduling jitter
- Sleep-based backpressure is approximate and wastes wakeups
- These loops make responsiveness depend on arbitrary time slices instead of actual events

### Recommended remediation

1. Replace the 20 ms polling loop with a blocking receive plus an explicit interrupt/shutdown path
2. Replace sink sleep backpressure with an event-driven mechanism:
   - condvar
   - channel
   - or another direct readiness notification
3. Preserve clean shutdown semantics; a blocking design still needs a reliable unblock path
4. Measure startup/resume latency before and after the change to validate the improvement

### Acceptance criteria

- [ ] The main playback worker loop no longer depends on 20 ms timeout polling
- [ ] Sink backpressure no longer uses fixed 5 ms sleeps
- [ ] Shutdown, pause, and resume remain responsive under the new signaling model

## Status

Open.
