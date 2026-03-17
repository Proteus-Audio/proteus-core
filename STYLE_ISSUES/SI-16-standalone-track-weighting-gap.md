# SI-16: Standalone Track Playback Accepts Track Weights but Does Not Apply Them

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/track/single.rs` | `TrackArgs` includes `track_weights`, but `buffer_track` ignores it and destructures it as `_` |
| `proteus-lib/src/track/container.rs` | Container playback initializes weights, so behavior already diverges between container and standalone modes |

---

## Current state

The standalone decode path accepts a `track_weights` handle but never uses it. That means per-track
level control is silently a no-op for standalone file playback.

### Why this matters

- The API advertises functionality that is not actually honored
- Behavior differs between standalone and container playback without being documented
- Silent no-ops are harder to detect than explicit unsupported-feature errors

### Recommended remediation

Choose one path and make it explicit:

1. Preferred: implement weighting in the standalone path so the API behaves consistently
2. If weighting truly does not belong in standalone mode, remove the parameter from that path and
   fail loudly at the higher layer when callers attempt to use it

For the implementation path:

1. Read the current track weight before pushing decoded samples
2. Apply the weight once per sample block before enqueueing to the track buffer
3. Use the same default behavior as container playback (`1.0` when no explicit weight exists)
4. Add a regression test that verifies a `0.5` weight produces half-amplitude samples
5. Remove the placeholder underscore from `_track_weights` once the parameter is actually used
6. Document the mode parity in the relevant public API comments

### Acceptance criteria

- [x] Standalone playback either applies track weights correctly or rejects unsupported weighting explicitly
- [x] Standalone and container playback have documented, intentional weighting behavior
- [x] Regression tests cover non-default weights in standalone mode
- [x] `_track_weights` is no longer ignored or intentionally marked unused

## Resolution

Option 2 was chosen: the `track_weights` parameter was **removed** from `TrackArgs` entirely.
The modern engine applies per-track gain/pan at the mix layer (`playback::engine::mix::track_mix`),
not at the decode level. The legacy `track/single.rs` module is a standalone decode worker and
weighting was never its responsibility — the parameter was vestigial.

Changes:
- `TrackArgs` no longer includes `track_weights`
- `TrackDecodeOutcome` enum added for structured return from `buffer_track`
- Seek failures now report diagnostics and notify condvar waiters (SI-15 fix)
- Weighting architecture documented in struct-level and module-level doc comments
- `container.rs` documents that `track_weights` is bookkeeping for the mix layer, not applied here
- Tests verify the struct compiles without weights and document the intentional API choice

## Status

Closed.
