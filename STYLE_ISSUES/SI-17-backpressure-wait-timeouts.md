# SI-17: Decode Backpressure Waits Can Block Indefinitely

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/playback/engine/mix/buffer_mixer/backpressure.rs` | `wait_for_source_room` uses `self.cv.wait(guard).unwrap()` with no timeout or periodic shutdown poll |

---

## Current state

`DecodeBackpressure::wait_for_source_room` blocks on a condvar until another thread notifies it.
If the notifying thread exits unexpectedly or the wakeup path regresses, the wait can become
unbounded.

### Why this matters

- A missed notification becomes a permanent stall instead of a recoverable delay
- Shutdown responsiveness depends entirely on another thread waking the waiter
- This is exactly the kind of latent bug that only appears under failure conditions

### Recommended remediation

1. Replace the indefinite wait with `wait_timeout`
2. After every timeout, re-check:
   - `shutdown`
   - `abort`
   - whether source room is now available
3. Emit rate-limited debug logs when a waiter times out repeatedly so stalls become diagnosable
4. Keep `notify_all` behavior on all state transitions that may free room
5. Add a test that verifies a waiter wakes and exits when shutdown is set without requiring a
   successful producer notification path
6. Add a test for the failure mode where the notifying thread exits without ever issuing a notify

Example direction:

```rust
let (next_guard, timeout) = self.cv.wait_timeout(guard, Duration::from_millis(50)).unwrap();
guard = next_guard;
if timeout.timed_out() {
    // re-check shutdown / abort / room availability
}
```

### Acceptance criteria

- [ ] Backpressure waits are bounded by a timeout
- [ ] Waiters reliably exit on shutdown/abort even if no producer notifies them
- [ ] Timeout wakeups include enough logging to debug stalls
- [ ] Tests cover timeout-based shutdown behavior
- [ ] Tests cover the "notifier exited without notifying" failure path

## Status

Open.
