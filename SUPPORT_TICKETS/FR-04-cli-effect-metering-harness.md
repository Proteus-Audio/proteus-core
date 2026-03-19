# FR-04: CLI Effect Metering Harness and Live Playback Visualization

## Summary

[`FR-01`](./FR-01-per-effect-level-metering.md) added library-side per-effect
level metering, analytical filter-response curves, and optional spectral
analysis. That work is still difficult to exercise outside an embedding app:
`proteus-cli` does not expose a stable way to enable the meters, run audio
through an effects chain for inspection, or visualize the per-effect
before/after snapshots.

Add an opt-in CLI metering surface for testing and debugging that can:

1. run an input through a configured effects chain without requiring normal
   interactive playback
2. enable FR-01 metering at runtime from the CLI
3. render each effect's **input vs output** levels in a human-readable
   before/after view in an offline report
4. emit machine-readable JSON so integration tests can assert on the meter
   output
5. optionally expose FR-01 spectral snapshots under a second CLI feature flag
6. render one live meter row per effect during the normal playback TUI, using
   the same player/runtime path as every playback format the CLI already
   supports

The default CLI build and default playback UX should remain unchanged.

---

## Motivation

The library already exposes the core inspection surface in
[`proteus-lib/src/playback/player/metering.rs`](../proteus-lib/src/playback/player/metering.rs):

- `set_effect_level_metering_enabled(...)`
- `set_effect_level_meter_refresh_hz(...)`
- `effect_levels()`
- `effect_frequency_responses(...)`
- `set_spectral_analysis_enabled(...)`
- `effect_band_levels()`

The CLI does not currently make practical use of those APIs:

- [`proteus-cli/src/cli/playback_runner.rs`](../proteus-cli/src/cli/playback_runner.rs)
  only shows the final output meter during normal playback
- [`proteus-cli/src/cli/verify.rs`](../proteus-cli/src/cli/verify.rs) only
  probes/decodes packets and never runs the DSP chain
- there is no deterministic, no-audio-device path for capturing per-effect
  meter output
- there is no live per-effect view in the normal playback UI even though the
  runtime player already exposes `effect_levels()`
- there is no test-friendly JSON report for asserting that an effect actually
  boosts, attenuates, or reshapes the signal as expected

That leaves FR-01 effectively GUI-only in practice, even though the library
support is already in place.

---

## Current Gap

### Compile-time gap

`proteus-cli/Cargo.toml` already forwards the library metering features:

```toml
effect-meter = ["proteus-lib/effect-meter"]
effect-meter-spectral = ["proteus-lib/effect-meter-spectral"]
```

Those flags compile the underlying APIs, but they do not define a CLI-facing
testing surface. There is no command today that consumes them.

### Runtime gap

Even in a build with `effect-meter` enabled:

- the CLI never calls `set_effect_level_metering_enabled(true)`
- the CLI never polls `effect_levels()`
- the CLI never renders per-effect input/output data
- the CLI never serializes that data for tests

### Testing gap

The current CLI integration tests in
[`proteus-cli/tests/runner_integration.rs`](../proteus-cli/tests/runner_integration.rs)
cover command dispatch and failure paths, but nothing can presently assert on
per-effect telemetry. Manual validation still requires writing ad hoc code or
embedding the library somewhere else.

---

## Proposed Design

### A. Add CLI-facing feature flags

Keep the existing pass-through flags, but add an explicit CLI harness layer on
top so metering commands and renderers remain off by default:

```toml
# proteus-cli/Cargo.toml
[features]
default = ["output-meter", "real-fft"]

# Existing low-level forwards; keep for compatibility.
effect-meter = ["proteus-lib/effect-meter"]
effect-meter-spectral = ["proteus-lib/effect-meter-spectral"]

# New CLI-facing testing/visualization features.
effect-meter-cli = ["effect-meter"]
effect-meter-cli-spectral = ["effect-meter-cli", "effect-meter-spectral"]
```

Rules:

- default CLI builds stay unchanged
- `effect-meter-cli` enables time-domain before/after reporting
- `effect-meter-cli-spectral` enables optional spectral reporting/visualization
- the command should still parse in non-meter builds and fail with a clear
  message, matching the current `bench` pattern

Example:

```text
Effect metering requires the `effect-meter-cli` feature.
```

This keeps the compile-time intent obvious: library metering support and
CLI metering tooling are related, but not the same concern.

### B. Add an explicit `meter` subcommand family

Add a new top-level CLI entry point dedicated to inspection rather than normal
playback:

```text
prot meter effects <INPUT>
  [--effects-json <PATH>]
  [--seek <SECONDS>]
  [--duration <SECONDS>]
  [--refresh-hz <HZ>]
  [--format table|bars|json]
  [--channels first-two|all|mixdown]
  [--summary final|max]
  [--spectral]
```

Recommended v1 behavior:

- run in an **offline/decode-only** mode by default
- apply the configured effect chain
- enable effect metering for the run
- capture per-effect snapshots without opening the normal playback TUI
- print a report and exit

This matters for testing: a metering command should not depend on a live audio
device, wall-clock timing, or manual key input.

### C. Enable effect metering during normal playback

The offline command is useful for deterministic testing, but the more important
CLI path is the normal playback session:

```text
prot <INPUT> [--effects-json <PATH>]
```

When built with `effect-meter-cli`, the playback runner should:

- enable `Player::set_effect_level_metering_enabled(true)` before playback
- poll `effect_levels()` during the existing TUI draw loop
- render one live meter row per effect in the active chain
- keep working for every playback format the CLI already supports:
  - standalone audio files
  - `.prot` / `.mka` containers
  - directory-backed sessions

The normal playback UI already has the final output meter. This FR extends that
surface with a dedicated per-effect pane, rather than forcing users into the
offline command whenever they want to inspect the live chain.

### D. Use FR-01's existing boundary semantics directly

For the CLI, "before/after metering of effects" should map directly to the
existing FR-01 data model:

- **before** = `EffectLevelSnapshot.input`
- **after** = `EffectLevelSnapshot.output`

The CLI should present those as one row or block per effect, using the active
effect names from the configured chain.

Recommended default table:

```text
idx  effect            in_peak  out_peak  delta  in_rms   out_rms  delta
0    Gain               -18.2     -12.2   +6.0   -24.0    -18.0   +6.0
1    LowPassFilter      -12.2     -15.1   -2.9   -18.0    -20.6   -2.6
```

Recommended bar view:

```text
[0] Gain
in : L [######      ] -18.2 dBFS | R [######      ] -18.1 dBFS
out: L [##########  ] -12.2 dBFS | R [##########  ] -12.1 dBFS
```

The key requirement is that the CLI view makes the effect boundary obvious:
what went in, what came out, and how much it changed.

### E. Add a deterministic report path for tests

The live `Player::effect_levels()` API stores the latest decimated snapshot,
which is fine for UI polling but too timing-sensitive for stable CLI tests.

The CLI harness should therefore build a deterministic report object for the
entire run, for example:

```rust
pub struct EffectMeterReport {
    pub sample_rate: u32,
    pub channels: usize,
    pub effects: Vec<EffectMeterRow>,
}

pub struct EffectMeterRow {
    pub effect_index: usize,
    pub effect_name: String,
    pub input_peak_dbfs: Vec<f32>,
    pub output_peak_dbfs: Vec<f32>,
    pub input_rms_dbfs: Vec<f32>,
    pub output_rms_dbfs: Vec<f32>,
}
```

Recommended summary modes:

- `final`: use the last stable snapshot from the run
- `max`: peak-hold / max-observed summary across the run for easier assertions

`--format json` should emit this report shape so CLI integration tests can
assert on real metering behavior without scraping text output.

### F. Build the CLI on a small library-side offline helper

`verify.rs` is too low-level for this because it only decodes packets and never
runs the Proteus DSP chain. The CLI should instead call a small library helper
dedicated to effect-meter inspection, for example:

```rust
proteus_lib::tools::effect_meter::run_report(config) -> EffectMeterReport
```

That helper should:

- decode audio
- build/apply the requested effects chain
- run the chain without a sink
- drive FR-01 metering internally
- return a deterministic report object to the CLI

This keeps the CLI thin and also makes the test surface reusable outside the
binary if needed.

### G. Optional spectral extension under the second feature flag

When built with `effect-meter-cli-spectral`, `prot meter effects --spectral`
should additionally expose the FR-01 spectral snapshots for supported filter
effects.

Recommended v1 output:

- keep the default time-domain before/after table
- append a compact per-effect bucket view for spectral input/output bands
- allow `--format json` to include the spectral section when compiled in

This should stay opt-in because the spectral path is materially more expensive
and depends on the FFT-enabled feature stack.

### H. Keep the initial scope bounded

This FR should not require a full redesign of the normal playback TUI.

Recommended non-goals for v1:

- no mandatory live playback overlay in the existing `prot <INPUT>` screen
- no attempt to meter both sides of `set_effects_inline` chain transitions
- no new public GUI-oriented API beyond what FR-01 already added

The priority is a reliable CLI testing harness and a readable before/after
visualization, not a second interactive workstation UI.

---

## Files Likely Affected

| File | Why |
| --- | --- |
| `proteus-cli/Cargo.toml` | Add `effect-meter-cli` and `effect-meter-cli-spectral` |
| `proteus-cli/src/cli/args.rs` | Register `meter` subcommand/options |
| `proteus-cli/src/cli/mod.rs` | Export new meter command module |
| `proteus-cli/src/cli/runner.rs` | Dispatch `meter` subcommand |
| `proteus-cli/src/cli/meter_cmd.rs` | Parse args, invoke library helper, render table/bars/JSON |
| `proteus-cli/tests/runner_integration.rs` | Add feature-gated meter command coverage |
| `proteus-lib/src/tools/mod.rs` | Export offline metering helper |
| `proteus-lib/src/tools/effect_meter.rs` | Offline effect-meter runner/report builder |

---

## Acceptance Criteria

- [x] `proteus-cli` exposes a `meter` command for effect metering inspection
- [x] default CLI builds remain unchanged and do not enable the new command path
      by default
- [x] builds without `effect-meter-cli` fail gracefully with a clear message
- [x] `prot meter effects ... --format table` shows per-effect before/after
      input/output levels
- [x] `prot meter effects ... --format bars` provides a readable terminal
      visualization of effect-boundary changes
- [x] `prot meter effects ... --format json` emits a deterministic report
      suitable for integration tests
- [x] the command can run without opening the normal playback UI or depending on
      a live audio device
- [x] `effect-meter-cli-spectral` optionally appends spectral bucket output for
      supported filter effects
- [x] normal playback (`prot <INPUT>`) enables live per-effect meters in the
      TUI when built with `effect-meter-cli`
- [x] the live per-effect meter pane uses the same playback path and therefore
      works across the file, container, and directory-backed input modes the
      player already supports

## Status

Done.
