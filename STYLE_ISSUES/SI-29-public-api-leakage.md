# SI-29: Internal Playback and Settings Types Still Leak Through the Public API

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/audio/buffer.rs` | `TrackBuffer` and `TrackBufferMap` are public aliases over `dasp_ring_buffer` internals |
| `proteus-lib/src/container/play_settings/mod.rs` | Versioned `PlaySettings*` models and aliases are public even though versioning is an internal deserialization detail |
| `proteus-lib/src/container/play_settings/legacy.rs` | Legacy settings structs are public for internal parsing rather than intentional API use |

---

## Current state

The roadmap's public-API item is only partly addressed. `container/prot` runtime planning types are
now `pub(crate)`, but other internal implementation types are still public.

### Why this matters

- Public type aliases over implementation details turn internal refactors into breaking changes
- Versioned settings structs expose an internal serde strategy as if it were stable API
- This increases the long-term compatibility burden of the crate for little user value

### Recommended remediation

1. Make `TrackBuffer` and `TrackBufferMap` `pub(crate)` unless there is a documented external use
   case that justifies stabilizing them
2. Make versioned/legacy `PlaySettings*` data models `pub(crate)` where possible
3. Preserve a smaller, intentional public API surface through higher-level accessors and wrapper
   types instead of raw internal schema types
4. Audit downstream callers in `proteus-cli` before reducing visibility

### Acceptance criteria

- [ ] `TrackBuffer` and `TrackBufferMap` are no longer public API unless explicitly justified and documented
- [ ] Versioned and legacy `PlaySettings*` parsing models are no longer exposed as stable public API
- [ ] CLI and internal callers compile against the reduced visibility surface

## Status

Open.
