# SI-32: DSP Guardrail Logic Is Still Duplicated Across Effects and Mixing Code

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/dsp/effects/*` | Gain clamping and channel-count guard patterns are duplicated across effect implementations |
| `proteus-lib/src/playback/engine/*` | Mixing/runtime code repeats some of the same sanitization logic |

---

## Current state

The roadmap still calls out duplicated NaN-safe gain clamping and channel-count `.max(1)` guards
spread across effect implementations and mix/runtime code.

### Why this matters

- Duplicate numerical guard logic drifts over time
- Shared DSP safety rules should be centralized to keep behavior consistent
- Utility extraction reduces both code volume and the risk of partial fixes

### Recommended remediation

1. Create a small `dsp::utils` module (or extend an existing utility module) for shared helpers
2. Move common patterns there, such as:
   - NaN-safe gain normalization/clamping
   - channel-count sanitization
   - any repeated finite-value guard logic
3. Replace duplicated local implementations with the shared helpers
4. Add focused tests around the shared helpers so edge-case behavior is defined once

### Acceptance criteria

- [ ] Shared DSP guardrail helpers exist in one intentional utility module
- [ ] Duplicate local implementations are removed or reduced to thin wrappers
- [ ] Tests define shared edge-case behavior for the extracted helpers

## Status

Open.
