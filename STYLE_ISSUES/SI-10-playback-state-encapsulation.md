# SI-10: Playback Runtime State Leaks Through Public Mutable Fields

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/playback/player/mod.rs` | `Player` exposes `finished_tracks` and `ts` as raw public lock-backed fields |
| `proteus-lib/src/playback/engine/mod.rs` | `PlayerEngine` exposes `finished_tracks` as a raw public lock-backed field |

Note: `PlayerEngineConfig` is intentionally a bag of `Arc<Mutex<_>>` handles — it is a
construction-time config struct (per the `Config` suffix convention) that transfers ownership of
shared-state handles from `Player` to the engine. This is not a violation.

---

## Current structure

`Player` is documented as the primary public controller, but it still exposes internal mutable
state directly:

- `pub finished_tracks: Arc<Mutex<Vec<i32>>>`
- `pub ts: Arc<Mutex<f64>>`
- `pub info: Info` — this one is lower risk since `Info` is read-only, but still leaks internals

`PlayerEngine` similarly exposes:

- `pub finished_tracks: Arc<Mutex<Vec<u16>>>`

Callers (including `proteus-cli`) access these by locking directly rather than through coordinated
`Player` methods.

### Specific style-guide violations

- Module state ownership is weakened by exposing implementation details directly
- The closure-update pattern is bypassed by handing out lock-backed fields publicly
- Callers must reason about the synchronization strategy to read timing or track-completion state

### Why this matters

These public fields couple callers to the exact lock layout. Reading `ts` requires the caller to
lock and unwrap; reading `finished_tracks` requires the same. Renaming, removing, or changing the
type of these fields is a breaking API change even though they are implementation details of the
playback engine. Replacing them with methods allows internal refactors without breaking callers.

### Recommended remediation

1. Make `Player.finished_tracks`, `Player.ts`, and `Player.info` private
2. Replace with focused accessor methods on `Player`:
   ```rust
   pub fn playback_position_secs(&self) -> f64 { ... }
   pub fn finished_track_indices(&self) -> Vec<i32> { ... }
   pub fn audio_info(&self) -> &Info { ... }
   ```
3. Make `PlayerEngine.finished_tracks` private; expose it through a method if callers need it
4. Update `proteus-cli` callers to use the new accessor methods
5. Stage this carefully — check all `player.finished_tracks` and `player.ts` call sites in
   `proteus-cli` before removing the public fields

### Acceptance criteria

- [ ] `Player.finished_tracks` and `Player.ts` are no longer public fields
- [ ] Read access to timing and completion state goes through `Player` methods
- [ ] `PlayerEngine.finished_tracks` is no longer a public field
- [ ] `proteus-cli` callers compile and behave correctly after the change
- [ ] Existing playback behaviors remain unchanged

## Status

Open.
