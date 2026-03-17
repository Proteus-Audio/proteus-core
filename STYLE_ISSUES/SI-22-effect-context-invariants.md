# SI-22: `EffectContext` Still Exposes Raw Public Fields Instead of Enforcing Invariants

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/dsp/effects/mod.rs` | `EffectContext` is public, all fields are public, and callers can construct arbitrary invalid values |

---

## Current state

`EffectContext` is a public struct with public fields:

- `sample_rate`
- `channels`
- `container_path`
- `impulse_response_spec`
- `impulse_response_tail_db`

Nothing prevents callers from constructing impossible or unsupported combinations such as
`channels = 0` or nonsensical sample-rate values.

### Why this matters

- Public mutable fields leak invariants into every caller
- DSP code should not have to defensively re-validate context structure everywhere
- The current shape makes future refactoring or validation changes harder to introduce
- This is one part of the roadmap's broader public-API-surface reduction work; other internal
  buffer and play-settings types still need separate visibility cleanup

### Recommended remediation

1. Make `EffectContext` fields private
2. Add a constructor or builder that validates required invariants:
   - `sample_rate > 0`
   - `channels >= 1`
   - `impulse_response_tail_db` within a sane supported range, if one exists
3. Expose read-only accessor methods for consumers that need the values
4. Keep cloning if it is genuinely needed, but ensure clones preserve only valid state
5. Audit all construction sites and migrate them to the validated constructor
6. Add tests for invalid-context rejection and valid-context creation

Example direction:

```rust
impl EffectContext {
    pub fn new(sample_rate: u32, channels: usize, ...) -> Result<Self, EffectContextError> { ... }
    pub fn sample_rate(&self) -> u32 { self.sample_rate }
}
```

### Acceptance criteria

- [x] `EffectContext` no longer exposes raw public fields
- [x] Construction flows through a validated constructor or builder
- [x] Call sites compile against accessors rather than direct field access
- [x] Tests cover both valid and invalid construction

## Status

Done.
