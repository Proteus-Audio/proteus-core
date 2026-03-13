# SI-30: Standalone and Container Decode Paths Still Implement EOS Logic Independently

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/track/single.rs` | Standalone decode path has its own seek/setup/finish semantics |
| `proteus-lib/src/track/container.rs` | Container decode path has separate EOS skew and finish handling |

---

## Current state

The two decode paths still implement end-of-stream and shutdown behavior independently. The
container path has explicit EOS skew logic; the standalone path has simpler duration/finish logic.

### Why this matters

- Semantics can drift silently between standalone-file and container playback
- Alignment and finished-track behavior are core playback invariants
- Bugs here are difficult to spot because both code paths are valid in isolation

### Recommended remediation

1. Extract shared EOS/shutdown concepts into reusable helpers where the logic is genuinely common
2. Define which behaviors are intentionally mode-specific and document them
3. Add tests that exercise equivalent completion scenarios through both paths and compare behavior
4. Keep container-specific skew logic only where it is truly required by multi-track demuxing

### Acceptance criteria

- [ ] Shared EOS and shutdown behavior is factored into common helpers where appropriate
- [ ] Intentional differences between standalone and container decode paths are documented
- [ ] Tests cover parity of common completion scenarios across both modes

## Status

Open.
