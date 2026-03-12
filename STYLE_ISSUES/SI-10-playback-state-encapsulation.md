# SI-10: Playback Runtime State Leaks Through Public Mutable Fields

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/playback/player/mod.rs` | `Player` exposes internal shared state such as `finished_tracks` and `ts` as public fields |
| `proteus-lib/src/playback/engine/mod.rs` | `PlayerEngine` exposes `finished_tracks`; `PlayerEngineConfig` exposes raw shared state handles |

---

## Current structure

`Player` is documented as the primary public controller, but it still exposes internal mutable
state directly:

- `pub info: Info`
- `pub finished_tracks: Arc<Mutex<Vec<i32>>>`
- `pub ts: Arc<Mutex<f64>>`

`PlayerEngine` and `PlayerEngineConfig` expose similar raw internals, including shared lock-backed
state that callers can mutate without going through coordinated methods.

### Specific style-guide violations

- Module state ownership is weakened by exposing implementation details directly
- The closure-update pattern is bypassed by handing out lock-backed fields
- Public APIs expose mutable internals instead of stable accessor/mutator methods

### Why this matters

These public fields couple callers to the exact synchronization strategy and internal data layout.
That makes refactors harder, encourages direct lock manipulation outside the owning module, and
works against the style guide’s “module owns its internal state” rule.

### Recommended remediation

1. Make runtime internals private on `Player` and `PlayerEngine`
2. Replace public fields with focused accessors:
   - read-only snapshots for timing/info
   - dedicated mutation methods for state changes
   - closure-based update methods where batched mutation is required
3. Narrow `PlayerEngineConfig` so construction inputs are explicit configuration values, not a bag
   of externally owned locks
4. Stage this carefully because downstream callers may depend on the current fields

### Acceptance criteria

- [ ] `Player` no longer exposes raw `Arc<Mutex<_>>` runtime fields publicly
- [ ] `PlayerEngine` no longer exposes raw mutable runtime fields publicly
- [ ] Public callers interact through methods or snapshot types instead of direct locks
- [ ] Existing playback behaviors remain unchanged after encapsulation

## Status

Open.
