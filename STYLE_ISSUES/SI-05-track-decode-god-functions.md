# SI-05: Track Decode — God Functions

## Files affected

| File | Function | Lines |
|---|---|---|
| `proteus-lib/src/track/container.rs` | `buffer_container_tracks` | ~270 |
| `proteus-lib/src/track/single.rs` | `buffer_track` | ~165 |

Both functions significantly exceed the 80-line hard limit. Each one performs a full
decode pipeline — format probing, decoder setup, seek, decode loop, sample processing,
and cleanup — in a single monolithic function body.

---

## `buffer_container_tracks` (~270 lines)

### What it does

The function (lines 33–304) does all of the following inside one `thread::spawn` closure:

1. Opens the container format via `symphonia`
2. Collects all audio track IDs from the format metadata
3. Creates per-track decoders and channel maps
4. Seeks each decoder to `start_time`
5. Runs the main decode loop: reads packets, routes by track ID, decodes, processes
   channels, pushes to per-track ring buffers
6. Handles `EndOfStream` and packet errors, marks tracks finished
7. Joins and marks any remaining tracks finished on exit

### Proposed extraction

Extract phases into private helpers within the same file:

```rust
/// Open the container and build per-track decoders.
/// Returns `None` if the format cannot be opened or contains no audio tracks.
fn open_container(
    file_path: &str,
    start_time: f64,
) -> Option<(Box<dyn FormatReader>, Vec<TrackDecoder>)>

/// Core decode loop: read packets, decode, push samples to buffers.
/// Returns the set of tracks that are still unfinished on exit.
fn run_decode_loop(
    format: &mut dyn FormatReader,
    decoders: &mut [TrackDecoder],
    args: &DecodeLoopArgs,
    abort: &Arc<AtomicBool>,
) -> Vec<u16>

/// Mark all unfinished tracks as complete.
fn finish_remaining_tracks(
    remaining: Vec<u16>,
    finished_tracks: &Arc<Mutex<Vec<u16>>>,
)
```

Then `buffer_container_tracks` becomes a thin coordinator that spawns a thread and
calls these helpers in sequence.

### Struct to introduce

Introduce a `TrackDecoder` struct to hold the decoder + channel metadata that is
currently managed with parallel `Vec`s:

```rust
struct TrackDecoder {
    track_id: u32,
    track_key: u16,
    decoder: Box<dyn Decoder>,
    channels: u8,
}
```

---

## `buffer_track` (~165 lines)

### What it does

The function (lines 33–197) does all of the following inside one `thread::spawn` closure:

1. Opens the file
2. Locates the target audio track by `track_id` or selects the first
3. Creates a decoder for that track
4. Seeks to `start_time`
5. Runs the decode loop: reads packets, decodes, processes channels, pushes samples
6. Handles errors and marks the track finished on exit

### Proposed extraction

```rust
/// Locate and open the target track within the format reader.
/// Returns the track's codec params and a fresh decoder.
fn find_and_open_track(
    format: &dyn FormatReader,
    track_id: Option<u32>,
) -> Option<(u32, Box<dyn Decoder>)>

/// Core decode loop for a single track.
fn run_single_track_loop(
    track_key: u16,
    channels: u8,
    format: &mut dyn FormatReader,
    decoder: &mut dyn Decoder,
    buffer_map: &TrackBufferMap,
    buffer_notify: Option<&Arc<Condvar>>,
    finished_tracks: &Arc<Mutex<Vec<u16>>>,
    abort: &Arc<AtomicBool>,
)
```

Then `buffer_track` becomes a coordinator: open → find track → seek → run loop → mark finished.

---

## Notes

- Neither refactor changes the public API or the calling convention — `buffer_track` and
  `buffer_container_tracks` remain the public entry points with their current signatures.
- All helper functions should be private (`fn`, not `pub fn`).
- The `TrackDecoder` struct (if introduced) is `container.rs`-private.
- Existing tests should continue to pass without modification.

## Acceptance criteria

- [ ] All existing tests pass (`cargo test -p proteus-lib`)
- [ ] `cargo check --all-features` shows no new errors or warnings
- [ ] `buffer_track` ≤80 lines
- [ ] `buffer_container_tracks` ≤80 lines
- [ ] Each extracted helper ≤40 lines
