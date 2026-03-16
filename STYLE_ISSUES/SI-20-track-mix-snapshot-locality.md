# SI-20: Track Mixing Still Performs HashMap Lookups in the Inner Sample Loop

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/playback/engine/mix/track_mix.rs` | `mix_active_tracks` and `mix_fading_tracks` look up per-track weights and channel gains from `HashMap` snapshots during hot mixing work |

---

## Current state

The mix helpers receive `HashMap<u16, f32>` and `HashMap<u16, Vec<f32>>` snapshots, then resolve
weight and channel-gain slices per track while doing per-chunk sample mixing. The snapshot itself
is correct, but the data layout is still lookup-oriented rather than iteration-oriented.

### Why this matters

- The inner mixing path should favor dense, indexable structures over hash lookups
- A stable per-chunk active-track set is a good fit for pre-resolved arrays
- Simplifying the data layout can also make later SIMD or vectorized work easier

### Recommended remediation

1. Build a chunk-local resolved track descriptor list before mixing:
   ```rust
   struct ResolvedTrackMix<'a> {
       track_key: u16,
       buffer: &'a TrackBuffer,
       weight: f32,
       channel_gains: &'a [f32],
   }
   ```
2. Convert the `HashMap` snapshots into this dense list once per chunk or whenever the active set
   changes
   The roadmap's preferred concrete implementation is "parallel `Vec`s indexed by position", which
   is a valid alternative if it proves simpler than a descriptor struct.
3. Keep the actual sample loop working over slices and scalars only
4. Reuse the same resolved descriptor shape for both active and fading mixes where possible
5. Add a benchmark or at least a focused regression test for parity with existing output

### Acceptance criteria

- [ ] Hot mixing loops no longer depend on per-track `HashMap` resolution
- [ ] Track weights and channel gains are pre-resolved into dense per-chunk structures
- [ ] Mixing output remains bitwise or numerically equivalent to the current implementation

## Status

Open.
