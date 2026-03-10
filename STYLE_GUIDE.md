# Proteus Style Guide

Canonical rules for all development in `proteus-core`. These rules are derived from patterns already present in the repository — the goal is to codify what works and eliminate the inconsistencies that remain.

---

## 1. Core Principles

- **Small, focused modules.** A file should do one thing. When a module grows large, split it into a directory with submodules.
- **Explicit over implicit.** Prefer named types over tuples, named constants over magic numbers, and explicit error variants over stringly-typed errors.
- **No panics in library code.** `proteus-lib` must be embeddable. Propagate errors; reserve panics for truly impossible invariants.
- **Real-time paths are sacred.** The mix loop runs on a dedicated thread. No allocation, no blocking, no locks in the hot path without explicit justification.
- **Tests travel with code.** Inline `#[cfg(test)]` modules, not separate test files.

---

## 2. Project Structure

### Workspace layout

```
proteus-core/
├── proteus-lib/      # Core library — container parsing, playback, DSP
├── proteus-cli/      # CLI binary backed by proteus-lib
└── proteus-scripts/  # Standalone utilities
```

### Module layout inside `proteus-lib`

Top-level modules map to architectural concerns:

| Module         | Purpose                                      |
| -------------- | -------------------------------------------- |
| `container/`   | `.prot`/`.mka` parsing, settings, scheduling |
| `playback/`    | Player API, engine, mix loop, decode workers |
| `dsp/effects/` | `AudioEffect` variants and DSP state         |
| `track/`       | Decode threads and ring buffer push          |
| `audio/`       | Shared ring buffer and sample primitives     |
| `diagnostics/` | Optional metrics reporter                    |
| `peaks/`       | Waveform peak extraction                     |
| `tools/`       | Shared decode and timer utilities            |
| `logging/`     | Log buffering and debug file helpers         |

### Module boundaries

- A module owns its internal state. Do not reach across module boundaries for implementation details — use the public or `pub(crate)` interface.
- Cross-cutting dependencies flow **down** (e.g., `playback` may use `dsp`; `dsp` must not use `playback`).
- `proteus-cli` may depend on `proteus-lib`, never the reverse.

### File splitting

When a module grows beyond ~400 lines, split it into a directory:

```
dsp/effects/convolution_reverb/
├── mod.rs           # Public surface, re-exports
├── reverb.rs        # Effect struct and DspEffect impl
├── impulse_response.rs
└── convolution.rs
```

`mod.rs` re-exports everything the parent module needs; callers import from the directory, not the subfiles.

---

## 3. File and Function Size Limits

| Unit              | Recommended | Hard limit |
| ----------------- | ----------- | ---------- |
| File              | ≤ 400 lines | 600 lines† |
| Function / method | ≤ 40 lines  | 80 lines   |
| `impl` block      | ≤ 200 lines | —          |
| Test module       | ≤ 150 lines | —          |

†`prot.rs` currently exceeds this; it is a known exception and a refactor target. Do not add to it without splitting existing code out.

Functions that exceed 40 lines are usually doing more than one thing. Extract helper functions rather than adding length.

---

## 4. Naming Conventions

| Item                     | Convention             | Example                                           |
| ------------------------ | ---------------------- | ------------------------------------------------- |
| Files / modules          | `snake_case`           | `prot_settings.rs`, `buffer_mixer/`               |
| Structs / enums / traits | `PascalCase`           | `PlayerState`, `AudioEffect`, `DspEffect`         |
| Enum variants            | `PascalCase`           | `Playing`, `Paused`, `EndOfStream`                |
| Functions / methods      | `snake_case`           | `build_runtime_instance_plan`                     |
| Constants                | `SCREAMING_SNAKE_CASE` | `DEFAULT_THRESHOLD_DB`, `OUTPUT_METER_REFRESH_HZ` |
| Unused bindings          | Leading `_`            | `let _track_weights = ...;`                       |

### Type-name suffixes

Use consistent suffixes to signal a type's role:

| Suffix     | Role                         | Example                              |
| ---------- | ---------------------------- | ------------------------------------ |
| `Settings` | Plain data, serializable     | `CompressorSettings`, `GainSettings` |
| `Effect`   | Settings + runtime state     | `CompressorEffect`, `GainEffect`     |
| `State`    | Ephemeral runtime state      | `CompressorState`, `PlayerState`     |
| `Error`    | Error enum                   | `ProtError`, `PlayerInitError`       |
| `Config`   | Constructor configuration    | `PlayerEngineConfig`                 |
| `Plan`     | Pre-computed scheduling data | `RuntimeInstancePlan`                |

### Function prefixes

| Prefix          | Meaning                                |
| --------------- | -------------------------------------- |
| `try_`          | Returns `Result`, may fail             |
| `build_`        | Constructs a value from parts          |
| `set_` / `get_` | Simple accessors on shared state       |
| `update_`       | Takes a closure to mutate shared state |

---

## 5. Function and Type Design

### Functions

- **One responsibility per function.** If the name needs "and", split it.
- **Prefer ≤ 4 parameters.** Bundle related parameters into a struct when you need more.
- **No side effects through shared mutable globals.** Pass state explicitly.
- **Pure helper functions** (no `&self`) belong at the bottom of the file, below `impl` blocks.

### Effect types

All DSP effects follow this structure, in order:

```rust
// 1. Constants (SCREAMING_SNAKE_CASE, grouped by purpose)
const DEFAULT_THRESHOLD_DB: f32 = -18.0;

// 2. Settings struct (serializable, Clone, Debug, Default)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CompressorSettings { ... }

impl CompressorSettings { ... }
impl Default for CompressorSettings { ... }

// 3. Effect struct (settings + runtime state, state is #[serde(skip)])
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CompressorEffect {
    pub enabled: bool,
    #[serde(flatten)]
    pub settings: CompressorSettings,
    #[serde(skip)]
    state: Option<CompressorState>,
}

// 4. Manual Debug impl (exclude opaque state)
impl std::fmt::Debug for CompressorEffect { ... }

// 5. DspEffect trait impl
impl DspEffect for CompressorEffect {
    fn process(&mut self, ...) -> Vec<f32> {
        if !self.enabled { return samples.to_vec(); }
        // ...
    }
    fn reset_state(&mut self) { ... }
}

// 6. Tests
#[cfg(test)]
mod tests { ... }
```

When adding a new effect, follow this layout exactly.

### Closures for shared-state mutation

Use the closure-update pattern for settings guarded by a `Mutex` or `Arc`:

```rust
player.update_buffer_settings(|s| {
    s.start_buffer_ms = 200.0;
});
```

Do not expose the lock directly.

---

## 6. Imports and Dependencies

### Ordering

Group `use` statements in this order, separated by blank lines:

1. `std` crate
2. External crates (alphabetical within group)
3. `log` macros (`use log::{debug, error, info, warn};`)
4. `crate::` paths (alphabetical)
5. `super::` / `self::` paths

```rust
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use log::{info, warn};

use crate::container::prot::Prot;
use crate::dsp::effects::AudioEffect;

use super::{Player, PlayerState};
```

### Dependency rules

- `proteus-lib` must not depend on `proteus-cli`.
- `dsp` must not depend on `playback` or `container`.
- Feature flags (`bench`, `debug`, `real-fft`) must be additive — they must not change public API surface.
- New external crate dependencies require justification. Prefer the standard library or existing dependencies.

---

## 7. Error Handling

### In `proteus-lib`

Define typed error enums. Implement `Display` and `Error` manually (or via `thiserror`):

```rust
#[derive(Debug)]
pub enum PlaySettingsLoadError {
    OpenFile(std::io::Error),
    ParseJson(serde_json::Error),
    MissingAttachment,
}

impl std::fmt::Display for PlaySettingsLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenFile(e) => write!(f, "could not open settings file: {e}"),
            Self::ParseJson(e) => write!(f, "could not parse settings: {e}"),
            Self::MissingAttachment => write!(f, "settings attachment not found"),
        }
    }
}

impl std::error::Error for PlaySettingsLoadError { ... }
```

- Return `Result<T, SpecificError>` from public functions, not `Result<T, Box<dyn Error>>`.
- Use `?` for propagation. Never swallow errors silently.
- Do not use `.unwrap()` or `.expect()` in library code unless the invariant is proven and documented with a comment.

### In `proteus-cli`

Use `anyhow::Result` throughout. Display errors in lowercase:

```rust
Err(err) => {
    error!("{}", err.to_string().to_lowercase());
    -1
}
```

### Panics

- `proteus-lib`: panics are forbidden except in `Default` impls and invariant-proven contexts (document why).
- `proteus-cli`: panics acceptable for unrecoverable startup failures.
- Never use `.unwrap()` on `Mutex::lock()` — prefer `unwrap_or_else` with a descriptive message if you must use a lock in a non-`Result` context.

---

## 8. Logging

Use the `log` crate everywhere. Never use `println!` or `eprintln!` in library code.

| Level    | When to use                                              |
| -------- | -------------------------------------------------------- |
| `error!` | Unrecoverable failures, unknown states                   |
| `warn!`  | Recoverable issues, skipped operations, fallbacks        |
| `info!`  | Significant state transitions, successful initialization |
| `debug!` | Developer trace information, gated on `--features debug` |

Do not use `trace!`.

Log messages should be lowercase and sentence-like, without a trailing period:

```rust
info!("decoded track {} buffer format: {}", track_id, label);
warn!("no supported audio track found in '{}'", path);
error!("unknown file format");
```

---

## 9. Documentation

### Module-level doc comments (`//!`)

Every public module file requires a `//!` doc comment describing its purpose and, for larger modules, its submodule layout:

```rust
//! High-level playback controller for the Proteus library.
//!
//! `Player` is the primary integration point for consumers that need to load a
//! container or file list, control transport state, and inspect DSP/runtime
//! telemetry. Implementation details are split into focused submodules:
//! - `controls`: transport operations and lifecycle orchestration.
//! - `effects`: DSP-chain and metering controls.
```

### Public item doc comments (`///`)

All `pub` functions, types, and constants in `proteus-lib` require a doc comment. Use structured sections where applicable:

```rust
/// Load the play-settings from the embedded attachment in a Matroska file.
///
/// # Arguments
///
/// * `file_path` - Absolute or relative path to the `.mka` container.
///
/// # Returns
///
/// A deserialized [`PlaySettingsFile`] on success.
///
/// # Errors
///
/// Returns [`PlaySettingsLoadError`] if the file cannot be opened, the
/// attachment is missing, or the JSON is malformed.
pub fn load_from_mka(file_path: &str) -> Result<PlaySettingsFile, PlaySettingsLoadError> { ... }
```

Sections to include:

- `# Arguments` — when parameters are non-obvious
- `# Returns` — when the return type needs clarification
- `# Errors` — whenever the function returns `Result`
- `# Panics` — whenever the function can panic

### Inline comments

Use inline comments only for non-obvious logic. If a comment is needed to explain _what_ code does, rename the variable or function instead. Comments should explain _why_:

```rust
// Keep schedule entries rectangular by carrying the previous value for
// missing slots, so downstream code can assume all rows are the same width.
```

Do not comment out code. Delete it — version control preserves history.

---

## 10. Testing

### Placement

Tests live in an inline module at the bottom of the source file they test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // helpers first, then tests
    fn make_settings() -> CompressorSettings { ... }

    #[test]
    fn disabled_effect_is_passthrough() { ... }
}
```

No separate `tests/` integration test directories unless testing cross-crate behaviour.

### Test function naming

Name tests as `{subject}_{condition}_{expected_outcome}` or a readable variant of it:

```rust
fn gain_disabled_passthrough()
fn sanitize_level_clamps_above_maximum()
fn buffer_track_marks_finished_when_open_fails()
fn crate_exports_expected_top_level_modules()
```

Avoid generic names like `test_gain` or `test1`.

### Assertions

- Use `assert_eq!` for exact values.
- Use `assert!((value - expected).abs() < 1e-6)` for floating-point comparisons.
- Prefer one logical assertion per test; split large tests into focused ones.
- Do not assert internal implementation details — assert observable behaviour.

### Test helpers

Define helper functions inside the `tests` module, not as public utilities:

```rust
#[cfg(test)]
mod tests {
    fn make_prot(paths: &[&str]) -> Prot { ... }
    fn assert_near(a: f32, b: f32) { assert!((a - b).abs() < 1e-6); }
}
```

### What to test

- Public API contracts and return values.
- Edge cases: empty input, zero values, boundary clamps, error paths.
- Do not test private implementation details that are likely to change.
- Every new public function in `proteus-lib` should have at least one test.

---

## 11. Code Smells to Avoid

| Smell | Why | What to do instead |
|---|---|---|
| `.unwrap()` / `.expect()` in lib | Panics in embedded contexts | Propagate with `?` or return `Result` |
| `println!` / `eprintln!` in lib | Not captured by TUI log buffer | Use `log` macros |
| Magic numbers | Unreadable and hard to change | Named `const` at top of scope |
| Struct fields named `data`, `info`, `state` without context | Too generic | Use specific names: `buffer_samples`, `track_metadata`, `compressor_state` |
| Functions that return `()` and mutate many things | Hard to test and trace | Split responsibilities or return values |
| `#[allow(dead_code)]` | Hides unused code | Delete the code; git history preserves it |
| `TODO` comments | Deferred work rarely gets done | File an issue; finish the work or delete the comment |
| Duplicate constants across modules | Diverges over time | Define once in the owning module, re-export or import |
| Allocation in the mix loop | Causes audio glitches | Pre-allocate at engine startup |

---

## 12. Example Good Patterns

### Small, self-contained effect module

`proteus-lib/src/dsp/effects/gain.rs` — 128 lines, single responsibility, complete test coverage, no external dependencies beyond serde.

### Structured error hierarchy

`proteus-lib/src/container/prot_settings.rs` — `PlaySettingsLoadError` with typed variants, full `Display` and `Error` impls, clear mapping from underlying errors.

### Closure-based settings mutation

`proteus-lib/src/playback/player/settings.rs` — `update_buffer_settings` accepts a closure, updates atomically under one lock, never exposes the lock directly.

### Module documentation

`proteus-lib/src/playback/player/mod.rs` — `//!` comment describes purpose, lists submodule responsibilities, and identifies the primary type for external consumers.

### Effect constants at top of scope

`proteus-lib/src/dsp/effects/diffusion_reverb/mod.rs` — all tuneable parameters as named constants at the top, grouped logically (defaults, limits, tuning arrays), making the algorithm legible without digging into function bodies.
