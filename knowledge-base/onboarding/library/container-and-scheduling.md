# Library Onboarding: Container and Scheduling Model

- [Back to Library Onboarding](./index.md)

## What This Part Solves

This layer answers:

- What tracks/sources exist?
- Which sources are active at playback start?
- When should shuffles happen?
- What effects and playback settings are declared in the container?

The main file is [`proteus-lib/src/container/prot.rs`](../../../proteus-lib/src/container/prot.rs).

## `Prot` Input Modes

`Prot` supports three shapes:

- Container path (`Prot::new`)
- Standalone grouped file paths (`Prot::new_from_file_paths`)
- Legacy nested file-path lists (`Prot::new_from_file_paths_legacy`)

In all cases, the output is a normalized runtime model with:

- `Info` metadata (sample rate, channels, durations)
- current track/path selection
- `shuffle_schedule`
- total duration

## What Happens During Construction

### Container-backed

`Prot::new(...)`:

1. Builds `Info`
2. Loads `play_settings.json` from Matroska attachments
3. Parses play settings versions (`legacy`, `v1`, `v2`, `v3`)
4. Calls `refresh_tracks()` to build active selections and schedule

### Directory/file-backed

`Prot::new_from_file_paths(...)`:

1. Normalizes and deduplicates file paths
2. Builds `Info` from files
3. Calls `refresh_tracks()` to build schedule from `PathsTrack` data

## `refresh_tracks()` Is the Key Rebuild Function

When shuffle state or track selections change, `refresh_tracks()` reconstructs:

- active track IDs or paths
- `shuffle_schedule`
- `duration`

This is important because higher layers (player/mix thread) consume the schedule as data, not logic.

## Shuffle Scheduling (Conceptual)

Internal schedule entries are timestamped source lists. The container layer expands "shuffle points" into concrete events so runtime playback can stay hot-loop friendly.

Key internal concepts in `prot.rs`:

- `ShuffleScheduleEntry`
- `ShuffleRuntimePlan`
- `RuntimeInstancePlan`
- `ActiveWindow`

This design lets the playback side do fast "what changes now?" checks rather than reparsing settings continuously.

## Play Settings and Effects

Container parsing also extracts effect configuration:

- `play_settings.json` may contain a serialized `Vec<AudioEffect>`
- Convolution reverb IR overrides (`ImpulseResponseSpec`) and tail settings can also be parsed

This means containers can carry both track scheduling and DSP chain configuration.

## Practical Debugging Tips

- If playback order or shuffling looks wrong, inspect `Prot::refresh_tracks()` and the `build_*_shuffle_schedule(...)` helpers first.
- If the first active track list is wrong, check the first `shuffle_schedule` entry (`at_ms = 0`).
- If durations look wrong, separate container metadata issues from decoded/scan-derived durations (`container::info`).

## Related Knowledge Base

- [Shuffle Points in Playback](../../player/shuffle-points-playback.md)
- [Player Data Flows](../../player/data-flows.md)
- [CLI Directory Playback Config Helpers](../cli/subcommands-and-project-files.md)
