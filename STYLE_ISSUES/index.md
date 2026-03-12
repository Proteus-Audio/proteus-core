# Style Issues

This directory tracks style violations that require non-trivial refactoring. Small
violations are fixed in-place; larger ones get a writeup here.

| Done | ID    | Writeup                                                                      | Summary                                                                                                                          |
| ---- | ----- | ---------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| [x]  | SI-01 | [SI-01-container-god-files.md](./SI-01-container-god-files.md)               | `prot.rs` (1 666 lines) and `info.rs` (817 lines) need splitting into focused sub-modules                                        |
| [x]  | SI-02 | [SI-02-mix-engine-god-files.md](./SI-02-mix-engine-god-files.md)             | `buffer_mixer/mod.rs` (944 lines) and `runner/mod.rs` (715 lines) need splitting                                                 |
| [x]  | SI-03 | [SI-03-player-god-files.md](./SI-03-player-god-files.md)                     | `worker/runner.rs` (893 lines), `player/mod.rs` (617 lines), and `controls.rs` (469 lines) need splitting                        |
| [x]  | SI-04 | [SI-04-dsp-god-files.md](./SI-04-dsp-god-files.md)                           | `multiband_eq.rs` (761 lines), `diffusion_reverb/mod.rs` (740 lines), and `convolution_reverb/mod.rs` (621 lines) need splitting |
| [x]  | SI-05 | [SI-05-track-decode-god-functions.md](./SI-05-track-decode-god-functions.md) | `buffer_container_tracks` (~270 lines) and `buffer_track` (~165 lines) exceed function-length limits                             |
| [x]  | SI-06 | [SI-06-spawn-mix-thread-god-function.md](./SI-06-spawn-mix-thread-god-function.md) | `spawn_mix_thread` (~536 lines) in `runner/mod.rs` exceeds function-length limits and keeps the file over the 600-line hard limit |
| [x]  | SI-07 | [SI-07-peaks-format-god-file.md](./SI-07-peaks-format-god-file.md)           | `peaks/format.rs` (690 lines) exceeds the 600-line hard limit and keeps peak IO, range math, resampling, and tests in one file |
| [x]  | SI-08 | [SI-08-public-api-doc-gaps.md](./SI-08-public-api-doc-gaps.md)               | `proteus-lib` still has 213 missing public-doc errors and cannot pass a `missing_docs` rustdoc gate                              |
| [x]  | SI-09 | [SI-09-library-mutex-lock-panics.md](./SI-09-library-mutex-lock-panics.md)   | `proteus-lib/src` still contains 185 `lock().unwrap()` calls, violating the no-panics/no-raw-lock-unwrap rule                   |
| [x]  | SI-10 | [SI-10-playback-state-encapsulation.md](./SI-10-playback-state-encapsulation.md) | `Player` and playback-engine types still expose raw mutable runtime state instead of owning it behind accessors                  |
