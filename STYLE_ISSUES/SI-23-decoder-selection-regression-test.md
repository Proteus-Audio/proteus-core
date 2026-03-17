# SI-23: Decoder Selection Fix Lacks Regression Coverage for Non-Audio First Tracks

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/tools/decode.rs` | Decoder-selection logic was fixed, but there is still no regression test for a container whose first track is not the first decodable audio track |

---

## Current state

The implementation now uses `find(|t| t.codec_params.codec != CODEC_TYPE_NULL)` instead of blindly
using `tracks()[0]`, but the roadmap's final acceptance item is still open: there is no fixture or
test that proves this remains correct.

### Why this matters

- This bug was subtle and container-layout dependent
- Without a targeted regression test, a future refactor can easily reintroduce `tracks()[0]`
- Decoder selection is a foundational helper used by multiple playback paths

### Recommended remediation

1. Add a focused test fixture or mock path containing:
   - a first track that is present but not decodable audio
   - a later track that is the correct audio track
2. Assert that `get_reader`/`get_decoder` succeeds and binds to the later decodable track
3. Add a negative test for the "all tracks null/unsupported" case if one is not already covered
4. Keep the test close to `tools/decode.rs` so future changes to track selection must update it

### Acceptance criteria

- [x] A regression test covers a container where the first listed track is not the first decodable audio track
- [x] The test fails if decoder selection regresses to `tracks()[0]`
- [x] Unsupported-track-only input still returns the typed `NoSupportedAudioTrack` error

## Status

Done.
