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
│   ├── mod.rs         # thin public surface: read_peaks_with_options, write_peaks_file
│   ├── header.rs      # Header, read_header, write_header, constants
│   ├── io.rs          # read_peaks_by_indices, peak row encoding/decoding helpers
│   ├── query.rs       # compute_requested_sample_range, compute_peak_range
│   ├── resample.rs    # time_align_peaks, downsample_peaks, average_reduce_channel
│   └── tests.rs       # inline-only helpers and format-focused tests
```

### Recommended extraction steps

1. Move header constants and `Header` parsing/serialization into `format/header.rs`
2. Move byte-level read/write loops into `format/io.rs`
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

- [ ] `cargo test -p proteus-lib peaks::` passes
- [ ] `cargo check --all-features` reports no new warnings
- [ ] Every file under `proteus-lib/src/peaks/format/` is `<= 400` lines
- [ ] `time_align_peaks` is reduced to `<= 80` lines or replaced by smaller helpers
- [ ] Binary file compatibility remains unchanged for existing `.peaks` files

## Status

Open.
