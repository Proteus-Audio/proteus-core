# SI-27: Single-Container Metadata Construction Still Forces Full Packet Scans

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/container/info/mod.rs` | `Info::new` still calls `get_durations_by_scan` directly instead of trying metadata-first probing |

---

## Current state

The multi-file path already uses `get_durations_best_effort`, but the single-container path still
does a full packet scan during `Info::new`.

### Why this matters

- Container open time becomes O(file size) even when duration metadata already exists
- Behavior is inconsistent between the single-container and multi-file constructors
- This slows CLI/library startup on large inputs for no user-visible benefit

### Recommended remediation

1. Change `Info::new` to try metadata/frame-count-based duration probing first, mirroring
   `new_from_file_paths`
2. Fall back to `get_durations_by_scan` only when metadata is missing or all-zero
3. Keep the existing explicit scan path available for callers or CLI flows that want guaranteed
   full-file accuracy
4. Document the behavior alongside any CLI `--read-durations` / `--scan-durations` switches

### Acceptance criteria

- [x] `Info::new` uses metadata-first duration probing
- [x] Full packet scans happen only as a fallback when metadata is insufficient
- [x] User-facing documentation reflects the metadata-vs-scan behavior

## Status

Done.
