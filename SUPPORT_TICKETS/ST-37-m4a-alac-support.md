# ST-37: Add Explicit `.m4a` ALAC Support

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/Cargo.toml` | Symphonia currently enables AIFF directly, while AAC/MP4 support is available indirectly through Rodio feature unification |
| `proteus-lib/src/tools/decode.rs` | Decode support checks should cover ALAC-in-M4A fixtures |
| `proteus-lib/src/container/info/duration/mp4.rs` | MP4/M4A duration probing should handle ALAC tracks as first-class supported audio |
| `proteus-cli/src/project_files.rs` | `.m4a` is already accepted as a supported audio extension, but support currently depends on codec contents |

---

## Current state

Proteus accepts `.m4a` as a supported audio file extension. AAC-in-MP4 is covered by Symphonia
`aac` + `isomp4` support, currently resolved through dependency feature unification. ALAC-in-M4A
requires Symphonia's `alac` codec feature and is not explicitly enabled.

This means an `.m4a` file may be accepted by discovery but still fail during probe/decode if the
contained codec is Apple Lossless rather than AAC.

### Why this matters

- `.m4a` is a container extension, not a codec guarantee
- Users commonly encounter both AAC `.m4a` and ALAC `.m4a`
- Support should be explicit in `proteus-lib` instead of depending on transitive `rodio` features
- Duration probing and decode support should agree on what `.m4a` variants are supported

## Recommended remediation

1. Make Symphonia format/codec features explicit in `proteus-lib/Cargo.toml` for direct Proteus usage:
   - `aiff`
   - `mp3`
   - `aac`
   - `isomp4`
   - `alac`

2. Add an ALAC-in-M4A fixture:
   - Prefer a small fixture under `test_audio/`
   - If a binary fixture is too large or licensing is unclear, add a deterministic fixture-generation note/script using `ffmpeg` when available
   - Keep the fixture short enough for normal test runs

3. Add decode/support regression coverage:
   - `check_audio_file_supported()` returns supported for ALAC `.m4a`
   - `verify decode` can decode at least one packet from the fixture
   - unsupported-codec errors remain clear for unknown MP4 audio codecs

4. Add duration regression coverage:
   - `get_duration_details()` returns a non-zero duration for ALAC `.m4a`
   - duration source is structural when MP4 timing atoms are present
   - duration does not fall back to stale free-form tags when reliable MP4 timing exists

5. Update user-facing documentation if supported file types are documented elsewhere:
   - Clarify that `.m4a` covers AAC and ALAC once this ticket is complete
   - Keep unsupported MP4 codecs out of the supported list unless explicitly tested

## Acceptance criteria

- [ ] `proteus-lib` explicitly enables Symphonia `alac`, `aac`, `isomp4`, and the other directly used format features instead of relying on transitive feature unification
- [ ] A small ALAC `.m4a` fixture or fixture-generation path exists
- [ ] Decode support tests pass for ALAC-in-M4A
- [ ] Duration probing reports a non-zero structural duration for ALAC-in-M4A
- [ ] `.m4a` support documentation distinguishes AAC/ALAC from unsupported MP4 audio codecs

## Status

Not started.
