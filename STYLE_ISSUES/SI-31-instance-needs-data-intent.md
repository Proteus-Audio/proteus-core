# SI-31: `instance_needs_data()` Name Still Hides Its Unconditional Behavior

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/playback/engine/mix/buffer_mixer/routing_helpers.rs` | `instance_needs_data` always returns `true` but reads like a conditional predicate |
| `proteus-lib/src/playback/engine/mix/buffer_mixer/mixing.rs` | Call sites imply a meaningful readiness decision that does not actually exist |
| `proteus-lib/src/playback/engine/mix/buffer_mixer/mod.rs` | Additional call sites preserve the misleading abstraction |

---

## Current state

The helper still exists solely to return `true` with a comment about strict alignment semantics.

### Why this matters

- The name suggests conditional routing logic that does not exist
- Misleading abstraction makes future maintainers search for nonexistent behavior
- This kind of semantic mismatch is cheap to fix and expensive to misunderstand

### Recommended remediation

1. Either remove the helper and inline `true` with a clear alignment comment at call sites
2. Or rename it to something explicitly unconditional, such as:
   - `alignment_requires_all_instances`
   - `instance_participates_unconditionally`
3. Keep the explanatory comment about strict alignment semantics whichever option is chosen

### Acceptance criteria

- [ ] The helper name and behavior match
- [ ] Call sites no longer imply hidden conditional logic
- [ ] The strict-alignment rationale remains documented

## Status

Open.
