# SI-01: Container Module — Oversized Files

## Files affected

| File | Lines |
|---|---|
| `proteus-lib/src/container/prot.rs` | 1 666 |
| `proteus-lib/src/container/info.rs` | 817 |

Both exceed the 400-line recommended limit by a large margin. Each file bundles several
distinct concerns that should live in separate, focused modules.

---

## `prot.rs` (1 666 lines)

### Current structure

The file contains all of the following in a single 1 666-line file:

- Core types: `Prot`, `ProtSource`, `ProtError`, `ShuffleSource`, `ShuffleScheduleEntry`,
  `ActiveWindow`, `RuntimeInstanceMeta`, `RuntimeInstancePlan` (lines 18–97)
- `Prot` impl block — constructor, `refresh_tracks`, `build_runtime_instance_plan`,
  accessor helpers (lines 98–750)
- `PathsTrack` struct + impl (lines 766–842)
- Two large private schedule builders: `build_id_shuffle_schedule` (69 lines, l.844),
  `build_paths_shuffle_schedule` (84 lines, l.915)
- 20+ private helper functions: slot layout, segment instances, combination counting,
  sanitize helpers, linked-slot lookup, source converters (lines 752–1283)
- Test module (lines 1285–1666)

### Proposed split

Convert `container/prot.rs` into a directory `container/prot/`:

```
container/prot/
├── mod.rs           # Re-exports Prot, ProtError, PathsTrack, RuntimeInstancePlan + friends
├── types.rs         # ShuffleSource, ShuffleScheduleEntry, ActiveWindow,
│                    # RuntimeInstanceMeta, RuntimeInstancePlan, PathsTrack
├── schedule.rs      # build_id_shuffle_schedule, build_paths_shuffle_schedule,
│                    # parse_shuffle_points, parse_timestamp_ms
│                    # + the slot-layout/segment/linked-slot helpers they call
└── helpers.rs       # sanitize_level, sanitize_pan, random_id, random_path,
                     # seconds_to_ms, count_*_combinations, sources_to_*,
                     # collect_legacy_tracks, versioned_tracks
```

`mod.rs` keeps the `Prot` impl (constructor, `refresh_tracks`, `build_runtime_instance_plan`)
and delegates to the sub-modules.

**Expected result**: `mod.rs` ~350 lines, each sub-file ≤300 lines.

### Functions that exceed the 40-line limit (must be refactored as part of this work)

| Function | Lines | Location |
|---|---|---|
| `build_runtime_instance_plan` | ~150 | l.446 |
| `build_paths_shuffle_schedule` | ~84 | l.915 |
| `build_id_shuffle_schedule` | ~69 | l.844 |
| `refresh_tracks` | ~64 | l.197 |

`build_runtime_instance_plan` in particular does three distinct things: expands grouped
schedule entries into concrete instances, computes active windows, and trims to `start_time`.
Each of those phases should be its own private function.

---

## `info.rs` (817 lines)

### Current structure

- `InfoError` enum + impls (lines 26–43)
- Public probe/duration API: `get_probe_result_from_string`, `get_durations`,
  `try_get_durations`, `get_durations_by_scan`, `try_get_durations_by_scan` (lines 45–258)
- Internal track-info gathering: `get_track_info`, `reduce_track_infos`,
  `gather_track_info`, `gather_track_info_from_file_paths`, `bits_from_*` helpers
  (lines 260–618)
- AIFF-specific fallback path: `AiffInfo`, `parse_aiff_info`, `extended_80_to_f64`,
  `fallback_track_info`, `fallback_durations` (lines 361–492)
- `Info` struct + impl (lines 620–675)
- Tests (lines 676–817)

### Proposed split

```
container/info/
├── mod.rs            # Info struct + impl, InfoError, public API re-exports
├── duration.rs       # get_durations, try_get_durations, get_durations_by_scan,
│                     # try_get_durations_by_scan, get_durations_best_effort,
│                     # gather_track_info*, reduce_track_infos
└── aiff.rs           # AiffInfo, parse_aiff_info, extended_80_to_f64,
                      # fallback_track_info, fallback_durations
```

**Expected result**: `mod.rs` ~200 lines, `duration.rs` ~350 lines, `aiff.rs` ~150 lines.

---

## Acceptance criteria

- [ ] All existing tests pass (`cargo test -p proteus-lib`)
- [ ] `cargo check --all-features` shows no new errors or warnings
- [ ] Each new file is ≤400 lines
- [ ] No function exceeds 80 lines
- [ ] Public re-exports in `mod.rs` preserve the existing import paths used by callers
