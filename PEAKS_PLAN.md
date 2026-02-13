Here’s a compact, stream-friendly **binary “peaks pyramid + tiles”** format that gives you **random access by time** (via tile lookups) and scales to **100k+ peaks at max zoom** (and way beyond). It’s mono-first but cleanly supports stereo/multi-channel later.

## Goals

* **Fast**: no JSON parse, minimal allocations.
* **Random access**: fetch only the time window you need.
* **Streamable to disk**: write data sequentially; patch small metadata at end.
* **Multi-channel**: works for mono/stereo/N channels.
* **Multi-resolution**: min/max “mipmap” levels for zoom.

---

## Data model

### Peak sample type

Each “peak” is a bucket summary of raw PCM samples:

* `min: i16`
* `max: i16`

So **4 bytes per channel per peak**.

For N channels:

* Interleave by channel for each peak index:

  * `peak0: (ch0 min,max)(ch1 min,max)...`
  * `peak1: (ch0 min,max)(ch1 min,max)...`

This makes it easy for the frontend to read a contiguous slice and render.

### Levels

Level 0 is your finest resolution (closest to raw), then each next level halves time resolution by combining two adjacent buckets:

* `min = min(min0, min1)`
* `max = max(max0, max1)`

(You can choose a different downsample factor, but 2× is simplest.)

### Tiles

Each level is split into fixed-size tiles:

* `tile_len_peaks` = e.g. **4096** peaks per tile (good default)
* Tile size bytes = `tile_len_peaks * channels * 4` (before compression)

Tiles are your unit of random access.

---

## File layout: “header + (tiles) + footer index”

This layout is **great for streaming** because you can write tiles sequentially without knowing final offsets up front, then write the index at the end.

```
[FixedHeader v1]   (fixed size)
[TileData ...]     (streamed, sequential)
[FooterIndex]      (written last)
```

### FixedHeader (example, 128 bytes)

All little-endian.

```
magic[8]        = "PKS1TILE"
version_u16     = 1
endianness_u8   = 1  (1 = little)
flags_u8        = bit0: tiles_zstd, bit1: tiles_gzip, bit2: uncompressed, etc. (choose one)
header_bytes_u32= 128

channels_u16
reserved_u16

sample_rate_u32
total_samples_u64      // original PCM sample count per channel
bucket_size_u32        // how many PCM samples per peak at level 0 (e.g. 256 or 512)
tile_len_peaks_u32     // e.g. 4096
num_levels_u16
reserved2_u16

index_offset_u64       // 0 in streaming write, patched at end
index_bytes_u64        // patched at end

// optional: a content hash or track id
track_id_hash_u64
reserved padding...
```

**Key fields for random access by time**

* `sample_rate`, `total_samples`, `bucket_size`, `tile_len_peaks`, `num_levels`

Given a time range, the client can compute which level & which tiles it needs.

### TileData section

Tiles are stored level-by-level, tile-by-tile (or any order you prefer), each preceded by a tiny tile header so readers can skip without consulting the index (optional but handy).

**Option A (fastest at runtime): fixed-size uncompressed tiles**

* No per-tile header needed if tile byte size is constant and you can compute offsets from index alone.
* Downsides: bigger files.

**Option B (recommended): per-tile compressed blocks**
Each tile stored as:

```
tile_level_u16
tile_index_u32        // 0..num_tiles(level)-1
uncompressed_bytes_u32
compressed_bytes_u32
payload[compressed_bytes_u32]
```

This adds overhead but keeps files small and lets you stream tiles out-of-order if you ever want to.

---

## FooterIndex (random access map)

Written at the end so you can stream tiles first.

```
index_magic[8]      = "PKS1IDX1"
num_levels_u16
channels_u16
tile_len_peaks_u32
bucket_size_u32
reserved_u32

// then per-level tables
for level in 0..num_levels:
  level_peaks_u64          // number of peaks in this level
  num_tiles_u32
  reserved_u32

  // offsets table (num_tiles entries)
  // each entry tells you where tile payload is in file + its compressed size
  entries[num_tiles]:
    file_offset_u64
    compressed_bytes_u32
    uncompressed_bytes_u32
```

This makes serving a tile trivial:

* seek to `file_offset`
* read `compressed_bytes`
* decompress into `uncompressed_bytes`
* interpret as `i16` min/max interleaved by channel

---

## Mapping “time → tiles” (random access by time)

Definitions:

* `bucket_size = PCM samples per peak at level 0`
* At level `L`, one peak spans: `bucket_size * 2^L` PCM samples
* One tile spans: `tile_len_peaks * bucket_size * 2^L` PCM samples

For a requested time window `[t0, t1]` in seconds:

1. Convert to sample indices:

   * `s0 = floor(t0 * sample_rate)`
   * `s1 = ceil(t1 * sample_rate)`

2. Pick a level based on viewport width in pixels `W`:

   * peaks needed at chosen level ≈ `W` (or `2W` if you oversample)
   * approximate level:

     * desired_peak_span_samples ≈ `(s1 - s0) / W`
     * pick smallest L such that `bucket_size * 2^L >= desired_peak_span_samples`
   * clamp `L` to `[0, num_levels-1]`

3. Compute peak indices at that level:

   * `p0 = s0 / (bucket_size * 2^L)`
   * `p1 = s1 / (bucket_size * 2^L)`

4. Compute tile range:

   * `tile0 = p0 / tile_len_peaks`
   * `tile1 = p1 / tile_len_peaks`

Fetch tiles `[tile0 .. tile1]` for that level.

**This is O(1) math and O(#tiles) I/O.**

---

## Rust write strategy (stream-friendly)

1. Open file, write `FixedHeader` with `index_offset=0,index_bytes=0`.
2. Generate level 0 peaks by streaming audio decode (or from your own PCM pipeline):

   * Accumulate `bucket_size` samples → emit one `(min,max)` peak.
   * Collect into a tile buffer until `tile_len_peaks`, then compress+write tile payload.
   * Record `(offset, compressed_bytes, uncompressed_bytes)` in a `Vec` for that level.
3. Build next levels by reading previous level peaks in a streaming way:

   * Combine pairs of peaks to produce next level peaks.
   * Again tile/compress/write, record offsets.
   * (You can do this without holding entire levels in memory by streaming from temp files or from the already-written file if your tile layout supports it; simplest is: write each level to a temp file first, then append.)
4. After all tiles written:

   * `index_offset = current_pos`
   * Write `FooterIndex` using your recorded tables.
   * `index_bytes = bytes_written_for_index`
5. Seek back to patch `FixedHeader.index_offset/index_bytes`.

Memory usage is basically:

* one tile buffer (uncompressed)
* one compression buffer
* the offset tables (tiny: ~16 bytes per tile)

---

## Multi-channel extension

Nothing changes except:

* `channels_u16 > 1`
* Uncompressed tile payload layout becomes:

  * For each peak `i` in tile:

    * For channel `c` in 0..channels:

      * `min_i16, max_i16`

You can later add:

* per-channel normalization flags
* optional RMS per bucket (another i16) if you want waveform “thickness”

---

## Why this is fast vs JSON

* The frontend can `fetch` an ArrayBuffer and interpret it with `DataView` / `Int16Array` (after decompression).
* No parsing, no string allocations.
* Random access is “tile math + one seek + one decompress”.

---

If you want, I can also:

* propose concrete defaults (bucket_size, tile_len_peaks, num_levels) for typical durations
* sketch a tiny reference implementation outline in Rust (writer + reader) and a JS decoder snippet (zstd/gzip + typed arrays).
