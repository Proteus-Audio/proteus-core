# FR-06: CLI Spectral Graphs for Spectral-Shaping Effects

## Summary

[`FR-04`](./FR-04-cli-effect-metering-harness.md) added live per-effect CLI
meters and an offline `meter` harness. [`FR-05`](./FR-05-audible-time-aligned-effect-metering.md)
fixed the timing semantics so the live effect meters track audible playback
time instead of the mix thread running ahead.

The live CLI meter pane still only shows scalar before/after level changes.
That is enough for gain staging, but it does not help much for
spectral-shaping effects such as:

- `LowPassFilter`
- `HighPassFilter`
- `MultibandEq`

Add a compact spectral graph to the CLI effect-meter UI for those effects so a
user can see the spectral shape change at a glance while listening, without
needing a GUI or reading raw bucket tables.

The default CLI build and default runtime behavior should remain unchanged.
This is specifically an interactive TUI enhancement, not a requirement to turn
on FFT analysis for non-visual playback paths such as `--quiet`.

---

## Motivation

The current live TUI answers only one question:

- "How much level changed across this effect?"

For filter-like effects, the more important question is often:

- "What part of the spectrum is this effect reshaping right now?"

Examples:

- a lowpass may barely change broadband peak level while still obviously
  removing high end
- a highpass may clean up rumble without a dramatic scalar dB delta
- a multiband EQ may boost one region and cut another, so one number hides the
  actual contour

The offline `prot meter effects --spectral` path already exposes spectral
bucket data, but it is currently a numeric dump rather than a readable graph,
and the normal playback TUI does not consume any of it.

The CLI should offer a small, terminal-friendly view that makes FR-01's
spectral analysis practical during ordinary playback.

---

## Current Gap

### A. Live CLI playback only renders level meters

[`proteus-cli/src/cli/ui.rs`](../proteus-cli/src/cli/ui.rs) currently renders
one compact level row per effect using `EffectLevelSnapshot` data from
[`proteus-cli/src/cli/playback_runner.rs`](../proteus-cli/src/cli/playback_runner.rs).

There is no live spectral rendering path in that pane.

### B. The runtime spectral API is not yet wired into the live TUI

The player already exposes runtime spectral snapshots through
[`Player::effect_band_levels()`](../proteus-lib/src/playback/player/metering.rs),
and the offline harness already consumes the same family of data in
[`proteus-lib/src/tools/effect_meter.rs`](../proteus-lib/src/tools/effect_meter.rs).

But today:

- `configure_live_effect_metering(...)` enables level metering only
- the playback draw loop never polls spectral snapshots
- the UI has no compact graph renderer for spectral-shaping effects
- the current playback path can run without a visible TUI (`--quiet`), so any
  future spectral enablement must follow actual UI visibility rather than
  blindly enabling FFT work for every playback session

### C. FR-05 audible-time semantics do not yet cover live spectral graphs

`FR-05` introduced an audible-time aligned accessor for level snapshots:

- `effect_levels()` for processing-time diagnostics
- `effect_levels_audible()` for live playback UI

There is no equivalent audible-time accessor for spectral snapshots yet. If
the CLI simply polled `effect_band_levels()` directly for live rendering, the
spectral graph could lead the heard audio by the same sink backlog that FR-05
already fixed for level meters.

---

## Desired Outcome

During normal CLI playback, supported spectral-shaping effects should show a
small per-effect spectral graph alongside the existing in/out level meter so a
listener can tell, in the live TUI:

- whether the effect is acting on lows, mids, or highs
- whether the effect is broadly cutting, boosting, or tilting the spectrum
- how the audible spectral shape changes when live effect edits are applied

This should remain compact enough for normal terminal sizes and should degrade
gracefully on narrow layouts.

This FR is about a small audio-reactive FFT view. It should not try to replace
the existing analytical `effect_frequency_responses(...)` API, which answers a
different question ("what is this filter configured to do?") and may still be
useful for future non-live CLI views.

---

## Proposed Design

### A. Extend the live metering path to include spectral snapshots

When built with `effect-meter-cli-spectral`, the CLI playback runner should
enable runtime spectral analysis in the same playback path that currently
enables level metering, but only when the effect-meter TUI pane is actually
visible.

- `set_spectral_analysis_enabled(true)`
- `set_spectral_analysis_refresh_hz(...)`

The live draw loop should then poll spectral snapshots alongside the existing
audible-time level snapshots.

Builds without `effect-meter-cli-spectral` should keep the current behavior and
should not allocate UI space for spectral graphs.

Recommended runtime rules:

- enable spectral analysis for the interactive TUI path only
- keep it disabled for `--quiet` playback and any future hidden/non-rendering
  mode
- prefer a lower default refresh cadence than scalar level meters in v1
  (for example 10-15 Hz) because FFT analysis is materially more expensive than
  the existing peak/RMS path

### B. Add an audible-time aligned spectral accessor

The live TUI should not read processing-time spectral snapshots directly.

Recommended API direction:

```rust
pub fn effect_band_levels(&self) -> Option<Vec<Option<EffectBandSnapshot>>>;
pub fn effect_band_levels_audible(&self) -> Option<Vec<Option<EffectBandSnapshot>>>;
```

Semantics:

- `effect_band_levels()` remains the latest processing-time snapshot for
  offline reports and diagnostics
- `effect_band_levels_audible()` returns the spectral snapshot aligned to the
  current audible playback boundary, matching the intent of `FR-05`

Implementation-wise, this should reuse the same timestamped publication model
introduced for audible-time level metering rather than inventing a second,
unrelated timing path.

More concretely, v1 should add audible-time storage for spectral snapshots as a
parallel path to the existing latest-only `spectral_snapshots` store:

- keep `effect_band_levels()` as the latest processing-time view
- add a timestamped spectral ring or a shared generic timestamped-snapshot
  helper for `effect_band_levels_audible()`
- publish spectral snapshots with the same mix-time stamp used for
  `effect_levels_audible()`
- retain the same graceful-degradation rule as FR-05 if a non-blocking publish
  attempt is skipped under contention

### C. Render a compact terminal graph per supported effect

The live effect pane should keep the current level row and append one compact
spectral row for effects whose spectral snapshots are available:

- `LowPassFilter`
- `HighPassFilter`
- `MultibandEq`

Recommended v1 rendering properties:

- terminal-friendly ASCII-only graph
- about 16-24 columns wide on ordinary terminals
- low-to-high frequency from left to right
- prefer a single delta-oriented row (`output_db - input_db`) in v1 so the
  panel stays compact; separate `in` / `out` rows can remain an offline-only or
  future wide-layout refinement
- cheap to render from already-available bucket snapshots

Example direction only:

```text
[1] LowPassFilter  in [######  ] -10.8  out [####    ] -16.4  d  -5.6
    spec d  : ....---====....
```

The exact glyph mapping can be tuned during implementation. The important point
is that a user can recognize a lowpass, highpass, or EQ contour without reading
raw numeric bucket lists. If color is available, it can reinforce sign
(`cut` vs `boost`), but the shape must remain legible in plain ASCII.

### D. Keep the layout bounded and predictable

The current effect meter panel is intentionally compact. This FR should extend
it without turning the playback TUI into a full workstation UI.

Recommended layout rules:

- only supported shaping effects consume an extra spectral row
- keep the existing cap on how many effects are shown at once
- preserve the current six-effect visible budget; the extra spectral row is per
  visible shaping effect, not a reason to show more effects overall
- if the terminal is too narrow, hide the spectral graph before hiding the
  existing scalar level meters
- if spectral data is still warming up, show a short placeholder instead of
  leaving the row blank

Recommended width policy for v1:

- below the current effect-meter minimum width, keep the whole effect pane
  hidden
- at widths that fit the existing scalar row but not a meaningful graph, keep
  the current level-only rendering
- only render the spectral row once there is enough width for a graph that can
  actually communicate shape

### E. Reuse the renderer in the offline CLI where practical

The offline `prot meter effects --spectral` command already prints bucket data,
but it currently does so as plain lists of center frequencies and dB values.

If the renderer can be shared cleanly, the same compact graph formatter should
also be used in:

- `--format bars` as the preferred first reuse site
- `--format table` only if it can be appended without making the numeric report
  hard to scan

This is secondary to the live TUI work, but it would keep the CLI spectral
language consistent across live and offline inspection paths.

### F. Keep the initial scope bounded

Non-goals for v1:

- no full-width spectrogram
- no scrolling frequency editor
- no mouse interaction or cursor probing
- no attempt to render spectral graphs for non-filter effects
- no requirement to expose precise per-band numeric labels in the live TUI
- no attempt to replace the analytical frequency-response path with this live
  FFT view

The goal is a small "shape at a glance" graph inside the existing meter pane,
not a terminal EQ editor.

---

## Files Likely Affected

| File                                                            | Why                                                                                                                   |
| --------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `proteus-lib/src/playback/effect_meter.rs`                      | Add audible-time storage/access for spectral snapshots alongside the existing processing-time store                   |
| `proteus-lib/src/playback/player/metering.rs`                   | Add `effect_band_levels_audible()` and document timing semantics                                                      |
| `proteus-lib/src/playback/engine/mix/runner/effect_metering.rs` | Publish timestamped spectral snapshots through the same audible-time path used by live level meters                   |
| `proteus-cli/src/cli/playback_runner.rs`                        | Enable runtime spectral analysis only when the TUI effect pane is visible, and poll audible-time spectral snapshots during draws |
| `proteus-cli/src/cli/ui.rs`                                     | Extend the effect meter pane with a compact spectral graph renderer and bounded layout rules                          |
| `proteus-cli/src/cli/meter_cmd.rs`                              | Optionally reuse the compact renderer for offline `meter --spectral` output                                           |

---

## Acceptance Criteria

- [ ] when built with `effect-meter-cli-spectral`, the normal playback CLI TUI
      shows a compact spectral graph for `LowPassFilter`, `HighPassFilter`, and
      `MultibandEq` effect rows
- [ ] the live spectral graph reflects audible playback time rather than the
      latest mix-thread snapshot
- [ ] non-spectral effects keep the current level-only rendering
- [ ] `--quiet` playback and other non-rendered paths do not enable runtime
      spectral analysis just because the binary was built with
      `effect-meter-cli-spectral`
- [ ] terminals that are too narrow for the graph degrade gracefully by hiding
      the spectral row first and preserving the existing level meter row
- [ ] builds without `effect-meter-cli-spectral` keep the current CLI behavior
      and do not enable runtime spectral analysis
- [ ] the spectral graph is readable without raw numeric bucket dumps and makes
      lowpass, highpass, and broad EQ contours obvious at a glance
- [ ] offline `prot meter effects --spectral` output can optionally reuse the
      same compact graph style without breaking the existing JSON report path or
      the existing numeric table semantics

## Status

Open.
