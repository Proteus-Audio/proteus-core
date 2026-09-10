# SI-07: Peaks Format Module Oversized and Multi-Responsibility

## Files affected

| File | Lines |
|---|---|
| `proteus-lib/src/peaks/format.rs` | 690 |

---

## `peaks/format.rs` (690 lines)

### Current structure

The file exceeds the 600-line hard limit from `STYLE_GUIDE.md` and mixes four different
responsibilities:

1. **Binary file IO**: `write_peaks_file`, `read_peaks_with_options`, `read_peaks_by_indices`
2. **Header serialization**: `Header`, `write_header`, `read_header`
3. **Range and resampling math**: `compute_requested_sample_range`, `compute_peak_range`,
   `time_align_peaks`, `downsample_peaks`, `average_reduce_channel`
4. **Format tests**: 240+ lines of round-trip, range, alignment, and validation tests

### Specific style-guide violations

- `proteus-lib/src/peaks/format.rs` is over the **600-line hard limit**
- `time_align_peaks` (lines 179-270) is ~92 lines, over the **80-line function hard limit**
- The module no longer does "one thing"; it owns serialization, query planning, resampling,
  and the entire test matrix in a single file

### Why this matters

Peak-file reading is now harder to change safely because each edit crosses binary format
parsing, query slicing, and resampling behavior. The current layout makes it too easy to
accidentally break one concern while touching another, and the large inline test module keeps
the file above the limit even if small helper extractions continue.

### Proposed split

Restructure `proteus-lib/src/peaks/` as a directory-led module:

```text
peaks/
├── mod.rs
├── error.rs
├── extract.rs
├── format/
│   ├── mod.rs         # thin coordinator: validates options, delegates, re-exports
│   ├── header.rs      # Header, read_header, write_header, constants
│   ├── io.rs          # write_peaks_file, read_peaks_by_indices, peak row encoding/decoding
│   ├── query.rs       # compute_requested_sample_range, compute_peak_range
│   ├── resample.rs    # time_align_peaks, downsample_peaks, average_reduce_channel
│   └── tests.rs       # inline-only helpers and format-focused tests
```

### Recommended extraction steps

1. Move header constants and `Header` parsing/serialization into `format/header.rs`
2. Move `write_peaks_file` and `read_peaks_by_indices` (and their byte-level helpers) into `format/io.rs`; re-export public functions from `format/mod.rs`
3. Move range selection helpers into `format/query.rs`
4. Split `time_align_peaks` into smaller helpers in `format/resample.rs`
   Suggested helpers:
   - `empty_aligned_channels`
   - `aligned_channel_peaks`
   - `aligned_bin_range`
   - `weighted_peak_average`
5. Keep `format/mod.rs` as a thin coordinator that validates options and delegates
6. Move the large test block into `format/tests.rs` so each implementation file stays focused

### Acceptance criteria

- [x] `cargo test -p proteus-lib peaks::` passes
- [x] `cargo check --all-features` reports no new warnings
- [x] Every file under `proteus-lib/src/peaks/format/` is `<= 400` lines
- [x] `time_align_peaks` is reduced to `<= 80` lines or replaced by smaller helpers
- [x] Binary file compatibility remains unchanged for existing `.peaks` files

### Validation notes

Final file sizes: `mod.rs` 75, `header.rs` 96, `io.rs` 98, `query.rs` 75, `resample.rs` 175, `tests.rs` 248 — all under 400.

`time_align_peaks` reduced to 36 lines (was 92). Replaced the inner bin loop with `aligned_bin_peak` (~25 lines) and the overlap-weighted accumulator with `weighted_peak_sum` (~22 lines), plus `empty_aligned_channels` and `aligned_channel_peaks` helpers.

10 peaks tests pass; zero new warnings.

## Status

Complete.
