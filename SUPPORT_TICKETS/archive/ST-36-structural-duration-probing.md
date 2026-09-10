# ST-36: Prefer Structural Duration Probing Over Stale Metadata Tags

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/container/info/mod.rs` | Duration probing currently mixes tag parsing, frame-count probing, format-specific fallbacks, and scan fallbacks |
| `proteus-lib/src/container/info/duration/ogg.rs` | Ogg page granule probing exists and should become part of a broader structural-duration probing strategy |
| `proteus-lib/src/container/info/aiff.rs` | AIFF already has a simple header fallback that should be folded into the same strategy |
| `proteus-cli/src/cli/playback_runner.rs` | `--read-durations` and `--scan-durations` output should describe the selected duration source clearly |

---

## Current state

Duration lookup can return stale tag metadata. The known example is
`test_audio/deep_trouble_000.ogg`, where the Vorbis-style `DURATION` tag says
`03:44:02.888000000` but the actual Ogg/Opus stream duration is `00:28:07.33`.

Ogg now has a fast structural probe that reads final page granule positions, but other supported
containers still rely on whatever Symphonia exposes or on format-specific ad hoc fallbacks.

### Why this matters

- User-facing duration can be wrong when editor metadata is stale
- Full packet scans are accurate but O(file size), making startup slow on large files
- Duration behavior is format-dependent but not documented as a clear priority order
- CLI flags do not expose whether a duration came from structure, tag metadata, frame counts, or a scan

## Recommended policy

Use this priority order everywhere duration is needed:

1. Prefer structural duration from the container, stream header, sample table, index, or final timestamp.
2. Use codec/frame-count headers when they are structural and not free-form metadata.
3. Use free-form tags such as `DURATION` only as a fallback.
4. Scan container frame/page headers when structure is missing but packet headers are cheap to walk.
5. Decode packets only as a last resort, and only for explicit scan/verify flows.

## Work items

1. [x] Introduce a focused duration probing module, e.g. `container/info/duration/`, with a result type that records:
   - track id
   - duration seconds
   - source kind: structural, frame count, tag, header scan, packet scan
   - whether the value is expected to be exact or approximate

2. [x] Route `get_durations`, `try_get_durations`, `get_durations_best_effort`, and CLI duration output through that module while preserving the existing public `HashMap<u32, f64>` API.

3. [x] Keep the current Ogg implementation as the model:
   - `.ogg` / `.opus`
   - read tail pages
   - use final non-negative granule position per stream serial
   - divide by the track sample rate
   - do not trust `DURATION` tags ahead of granule positions

4. [x] Add native FLAC structural probing:
   - read `STREAMINFO.total_samples`
   - divide by `STREAMINFO.sample_rate`
   - handle zero/unknown total samples by falling back to Symphonia/frame-count/tag/scan behavior
   - add fixtures or synthetic parser tests for present, zero, and malformed `STREAMINFO`

5. [x] Add WAV/RIFF structural probing:
   - for PCM and IEEE float, derive duration from `data` chunk size / block align / sample rate
   - handle extensible WAV where the format is still PCM/float
   - avoid using RIFF chunk sizes for compressed formats unless the codec has a reliable frame count
   - keep existing decode/scan fallback for unsupported compressed WAV variants

6. [x] Add AIFF/AIFC structural probing parity:
   - move existing `COMM` chunk parsing into the shared duration module
   - derive duration from sample frame count / sample rate
   - keep the existing track-info fallback behavior intact

7. [x] Add MP3 fast duration probing:
   - parse Xing/Info frame headers for total frame count
   - parse VBRI headers when present
   - for CBR files without a VBR header, estimate from file length, bitrate, and frame parameters
   - for VBR files without Xing/VBRI, scan MP3 frame headers without decoding audio
   - tag metadata remains lower priority than these structural/header-derived values

8. [x] Add MP4/M4A/AAC-in-MP4 duration probing:
   - derive track duration from `mdhd` timescale/duration
   - account for edit lists (`elst`) where they affect playable duration
   - handle sample timing tables (`stts`) when track duration is absent or suspicious
   - document fragmented MP4 limitations and fall back to Symphonia/scan behavior where needed

9. [x] Add Matroska/WebM/MKA duration probing:
   - prefer Segment Info `Duration` with `TimestampScale` when present
   - when missing or suspicious, seek near EOF and read the last cluster/block timestamp if practical
   - preserve `.prot`/`.mka` behavior and track-id mapping
   - add tests for stale tag-like metadata versus structural segment/cluster timing

10. [x] Add ADTS AAC header-scan fallback:
   - detect raw ADTS streams
   - scan ADTS frame headers and sum frame durations without decoding
   - use as a fallback when no container-level duration exists

11. [x] Add source reporting for diagnostics:
   - expose an internal helper that returns duration source details
   - update `--read-durations` to show the source when requested or in verbose mode
   - keep `--scan-durations` as the explicit expensive accuracy path

12. [x] Add regression coverage:
   - stale Ogg `DURATION` tag returns granule-derived duration
   - FLAC `STREAMINFO` beats tag metadata
   - WAV/AIFF structural frame counts beat tag metadata
   - MP3 Xing/VBRI beats tag metadata
   - MP4 `mdhd`/sample-table duration beats tag metadata
   - Matroska segment/cluster timing beats tag metadata
   - missing structural duration falls back without panicking

## Acceptance criteria

- [x] Duration probing has a documented priority order and shared result type
- [x] Existing public duration APIs continue to work
- [x] Ogg/Opus and Ogg/Vorbis use page granule positions ahead of tags
- [x] FLAC, WAV, AIFF, MP3, MP4/M4A, Matroska/WebM/MKA, and ADTS AAC have format-specific structural or header-scan probes
- [x] Free-form `DURATION` tags are never preferred over reliable structural duration
- [x] Full packet scans are only used as fallback or when explicitly requested
- [x] Tests cover stale metadata for every implemented format family
- [x] CLI diagnostics can identify how a duration was determined

## Status

Complete. The duration-probing implementation is in place across the listed format families,
with fixture coverage for the local FLAC/WAV/AIFF/MP3/Ogg/Matroska assets, synthetic parser
coverage for MP4 `mdhd`/`elst`/`stts`, Matroska last-cluster timing, and ADTS AAC, plus explicit
priority coverage proving stale tag durations lose to structural, frame-count, and header-scan
sources for every implemented format family.
