# FR-01: Per-Effect Input/Output Level Metering and Spectral Analysis

## Summary

Add three optional GUI-facing inspection capabilities for the DSP chain:

1. **Per-effect input/output level metering** for every effect in the active chain
2. **Analytical frequency-response curves** for filter-based effects
3. **Optional FFT-based spectral analysis** for frequency-shaping effects

This should be implemented so that:

- the feature is **compile-time opt-in**
- the public API remains **additive** across feature sets
- the hot mix path does **no extra steady-state work unless metering is enabled at runtime**
- the mix thread never blocks or allocates in the metering fast path

The current mix architecture already gives us the right hook point: `run_effect_chain()` processes the mix thread's **local** effect chain using pre-allocated ping-pong scratch buffers. Metering should attach there, not inside individual effects.

---

## Motivation

Today the only exported metering is the final `OutputMeter` in [`playback/output_meter.rs`](../proteus-lib/src/playback/output_meter.rs). There is no way for a GUI to inspect what is happening at each effect boundary.

GUI/editor consumers need three different views:

- **Per-effect I/O levels**: input peak/RMS and output peak/RMS so users can see gain staging through the chain
- **Analytical filter curves**: the theoretical response of LPF/HPF/EQ sections for static overlay rendering
- **Optional spectral analysis**: animated frequency-domain energy views when an EQ/filter editor is open

These use cases have very different cost profiles. They should not be forced through one always-on implementation.

---

## Compile-Time And Runtime Gating

### Compile-time gating

Use a new feature family that is **off by default**:

```toml
# proteus-lib/Cargo.toml
[features]
effect-meter = []
effect-meter-spectral = ["effect-meter", "real-fft"]

# proteus-cli/Cargo.toml
[features]
effect-meter = ["proteus-lib/effect-meter"]
effect-meter-spectral = ["proteus-lib/effect-meter-spectral"]
```

Recommended split:

- `effect-meter`: Tier 1 and Tier 2
- `effect-meter-spectral`: Tier 3 only

This keeps the cheap time-domain and analytical pieces available without forcing FFT support when a consumer does not want it.

### Public API rule

Per [`STYLE_GUIDE.md`](../STYLE_GUIDE.md), feature flags must be additive and must not change the public API surface. That means:

- public metering types should exist regardless of feature flags
- `Player` metering methods should exist regardless of feature flags
- disabled builds should return `None`, empty no-op snapshots, or ignore setters rather than removing methods with `#[cfg]`

This should follow the same pattern as [`playback/output_meter.rs`](../proteus-lib/src/playback/output_meter.rs): enabled and disabled implementations behind a stable API.

### Runtime gating

Compile-time gating alone is not enough. Background metering work should be runtime-disabled by default and only perform real work while a consumer explicitly wants it.

Recommended runtime API:

```rust
impl Player {
    pub fn set_effect_level_metering_enabled(&self, enabled: bool) { ... }
    pub fn set_effect_level_meter_refresh_hz(&self, hz: f32) { ... }

    pub fn set_spectral_analysis_enabled(&self, enabled: bool) { ... }
    pub fn set_spectral_analysis_refresh_hz(&self, hz: f32) { ... }
}
```

Rules:

- **Tier 1**: disabled by default at runtime
- **Tier 2**: on-demand only, so no background runtime gate is needed
- **Tier 3**: disabled by default at runtime
- when a tier is runtime-disabled, the mix thread should do no tier-specific work beyond a cheap gate check

---

## Design: three tiers

### Tier 1: Time-domain peak/RMS levels

**What it provides**

Per-effect input peak, input RMS, output peak, and output RMS, per channel.

**Correct hook point**

Hook this at [`run_effect_chain()`](../proteus-lib/src/playback/engine/mix/effects.rs), not inside each effect. That keeps individual effects unchanged and avoids per-effect branching.

**How it works**

```text
for each effect in chain:
    if level_metering_due:
        measure scratch_a -> input_levels[effect_index]

    effect.process_into(scratch_a, scratch_b, ...)
    swap/crossfade as normal

    if level_metering_due:
        measure post-effect buffer -> output_levels[effect_index]
```

For normal stages, the post-effect buffer is `scratch_a` after the swap.

For `EffectEnableFade`, the post-effect buffer is the **crossfaded** result in `scratch_a`, not raw `scratch_b`. The meter must reflect what the listener actually hears.

**Measurement**

The level pass iterates the interleaved samples once and tracks:

- peak: `abs(sample).max(...)`
- RMS: `sum(sample * sample)` followed by `sqrt(sum / frames_per_channel)`

This is cheap, but not free. It still scales with:

- number of effects
- number of measured boundaries
- refresh rate

That is why runtime gating and decimation are required.

**Refresh scheduling**

Do not schedule by “every N chunks”. Chunk sizes already vary with convolution batching, slicing, and drain paths.

Instead, schedule by **accumulated sample frames per channel**, the same way the existing output meter tracks time. Example:

- store `level_frame_samples_per_channel`
- accumulate processed frames per channel
- publish a new snapshot whenever the accumulated count crosses that threshold

This makes refresh cadence stable regardless of chunk size.

**Data structures**

```rust
#[derive(Debug, Clone, Default)]
pub struct LevelSnapshot {
    pub peak: Vec<f32>,
    pub rms: Vec<f32>,
}

#[derive(Debug, Clone, Default)]
pub struct EffectLevelSnapshot {
    pub input: LevelSnapshot,
    pub output: LevelSnapshot,
}
```

**Publishing to the control thread**

The current architecture matters here:

- the mix thread processes `local_effects`
- it does **not** hold the shared `effects` mutex during DSP anymore

So the metering data must **not** piggyback on the shared effects mutex.

Recommended initial transport:

- keep a dedicated shared metering store on the `Player`
- the mix thread publishes only on decimated metering ticks
- publication must be **non-blocking**

Practical v1 recommendation:

- store the latest snapshot behind a dedicated `Arc<Mutex<_>>`
- on the mix thread, publish with `try_lock()`
- if the lock is contended, drop that publication and keep running

That preserves the “no blocking in the hot path” rule while staying simple and dependency-free. A lock-free double-buffer is also acceptable, but it is not required for v1.

**Player API**

```rust
impl Player {
    /// Returns `None` when the feature is not compiled in or when runtime
    /// level metering is disabled.
    pub fn effect_levels(&self) -> Option<Vec<EffectLevelSnapshot>> { ... }
}
```

Using `Option` is clearer than returning an empty `Vec`, which would otherwise conflate:

- no effects in the chain
- runtime metering disabled
- feature not compiled

**Inline-transition semantics**

During `set_effects_inline` the engine may run **two chains in parallel** and crossfade between them. Per-effect metering is ambiguous during that window.

Define v1 behavior explicitly:

- keep the last stable per-effect snapshot during a full-chain inline transition
- resume publishing once the new local chain becomes active

Do not try to meter both chains in v1.

**Estimated steady-state cost**

- zero when the feature is not compiled
- effectively zero beyond a gate check when compiled but runtime-disabled
- one extra measurement pass at the configured refresh cadence when runtime-enabled

---

### Tier 2: Analytical frequency-response curves

**What it provides**

Static filter-response curves for:

- `LowPassFilterEffect`
- `HighPassFilterEffect`
- `MultibandEqEffect`

This is the classic EQ-curve overlay shown in editors.

**Important correction**

This should be computed from **effect settings**, not from the mix thread's mutable DSP state.

Reasons:

- the response is analytical, not sample-history-dependent
- the control-path copy of the effect chain already carries the latest settings
- biquad coefficient smoothing in the mix thread should not leak into UI queries
- computing from settings naturally shows the **target** curve during parameter ramps

Because of that, this does **not** need to be a `DspEffect` runtime hook. A plain `&self` path on `AudioEffect` or effect-specific helper methods is the better fit.

**How it works**

For a normalized biquad with coefficients `(b0, b1, b2, a1, a2)`:

```text
H(e^jw) = (b0 + b1 e^-jw + b2 e^-j2w) / (1 + a1 e^-jw + a2 e^-j2w)
```

Evaluate `|H(e^jw)|` at `N` log-spaced points, for example 128 points from 20 Hz to Nyquist.

For multiband EQ:

- compute one curve per configured section
- multiply linear magnitudes to get the composite response

“Per-band” here should mean **per configured filter section**:

- optional low edge
- each parametric point
- optional high edge

**Data structures**

```rust
/// A single point on a frequency response curve.
#[derive(Debug, Clone, Copy)]
pub struct FrequencyResponsePoint {
    pub freq_hz: f32,
    pub gain_db: f32,
}

/// Analytical frequency response for a filter-type effect.
#[derive(Debug, Clone)]
pub struct FilterResponseCurve {
    /// Composite response across all bands.
    pub composite: Vec<FrequencyResponsePoint>,
    /// Per-band response curves (for multiband EQ).
    /// Empty for single-filter effects (lowpass, highpass).
    pub per_band: Vec<Vec<FrequencyResponsePoint>>,
}
```

**Player API**

```rust
impl Player {
    /// Non-filter effects return `None` in their slot.
    pub fn effect_frequency_responses(
        &self,
        num_points: usize,
    ) -> Vec<Option<FilterResponseCurve>> { ... }
}
```

**When to compute**

On demand only. No mix-thread work, no background cache required for v1.

The calculation is cheap enough that caching is optional. If later profiling shows repeated UI polling overhead, cache by settings mutation generation on the control-path copy of the chain.

---

### Tier 3: FFT-based spectral analysis

**What it provides**

Animated, runtime-optional spectral energy views for frequency-shaping effects.

This is the expensive tier and should only exist when:

- `effect-meter-spectral` is compiled
- runtime spectral analysis is explicitly enabled

**What it is not**

It is not the same thing as Tier 2.

- Tier 2 shows the filter's **theoretical response**
- Tier 3 shows the **actual signal energy** in the audio passing through that effect

**How it works**

1. Identify the effect slots that need spectral analysis:
   - low-pass
   - high-pass
   - multiband EQ
2. While runtime spectral analysis is enabled:
   - accumulate input and output samples for those slots into pre-allocated analysis buffers
   - only do this accumulation when the runtime gate is on
3. At the configured spectral refresh cadence:
   - apply a Hann window
   - run a real FFT using `realfft`
   - reduce bins into UI-facing analysis buckets
   - publish the latest per-effect snapshots

**Channel policy**

Tier 3 should be explicit about channel handling. Recommended v1 behavior:

- accumulate **channel-aggregated power** for the analysis view
- expose one spectral snapshot per effect, not one per channel

This keeps the output compact and matches typical EQ-editor UI expectations.

**Bucket semantics**

The original “one band per configured EQ point” wording is too loose. For a parametric EQ, bands overlap and Q matters, so there is no single perfect non-overlapping partition.

Define v1 as a UI-friendly heuristic:

- for LPF/HPF, split the spectrum into two buckets at the cutoff frequency
- for multiband EQ, derive bucket boundaries from the sorted control frequencies:
  - midpoint between adjacent point frequencies
  - optional edge-filter cutoffs included as outer boundaries

This produces stable editor bars, but it should be documented as **analysis buckets aligned to the visible controls**, not exact isolated per-filter contributions.

If exact per-section contribution is needed later, that is a separate, more expensive feature.

**Data structures**

```rust
/// Per-band spectral energy for a single measurement direction.
#[derive(Debug, Clone, Default)]
pub struct BandLevels {
    /// Energy per band in dB. Length matches the number of configured bands.
    pub bands_db: Vec<f32>,
    /// Center frequency label for each band in Hz.
    pub band_centers_hz: Vec<f32>,
}

/// Spectral band levels for a single filter effect.
#[derive(Debug, Clone, Default)]
pub struct EffectBandSnapshot {
    pub input: BandLevels,
    pub output: BandLevels,
}
```

**Player API**

```rust
impl Player {
    /// Returns `None` when spectral support is not compiled in or when it is
    /// runtime-disabled.
    pub fn effect_band_levels(&self) -> Option<Vec<Option<EffectBandSnapshot>>> { ... }
}
```

**Runtime gating requirements**

When runtime spectral analysis is off:

- do not maintain per-effect ring buffers
- do not run FFTs
- do not compute band reductions
- do not publish spectral snapshots

Only a cheap gate check should remain.

To keep the disabled path cold, lazily initialize FFT plans and spectral buffers on first enable or first post-enable chain rebuild.

**Inline-transition semantics**

Same as Tier 1:

- freeze the last stable spectral snapshot during a full-chain inline transition
- rebuild analyzers when the new chain becomes active

**Estimated cost**

Reasonable for modern CPUs when enabled, but not negligible. That is why this tier needs both compile-time and runtime gating.

---

## Implementation Plan

### Phase 1: Infrastructure and Tier 1

**Files to create**

| File                                       | Purpose                                                         |
| ------------------------------------------ | --------------------------------------------------------------- |
| `proteus-lib/src/dsp/meter/mod.rs`         | Stable public metering types re-exported regardless of features |
| `proteus-lib/src/dsp/meter/level.rs`       | Peak/RMS measurement helpers                                    |
| `proteus-lib/src/playback/effect_meter.rs` | Enabled/disabled runtime store and publication helpers          |

**Files to modify**

| File                                                            | Change                                                         |
| --------------------------------------------------------------- | -------------------------------------------------------------- |
| `proteus-lib/src/dsp/mod.rs`                                    | Export `meter` unconditionally                                 |
| `proteus-lib/src/playback/mod.rs`                               | Add internal `effect_meter` module                             |
| `proteus-lib/src/playback/player/mod.rs`                        | Add shared effect-meter state to `Player`                      |
| `proteus-lib/src/playback/player/builder.rs`                    | Initialize effect-meter state                                  |
| `proteus-lib/src/playback/player/locks.rs`                      | Add recoverable lock accessor if a mutex-backed store is used  |
| `proteus-lib/src/playback/player/lifecycle.rs`                  | Reset effect-meter state on teardown                           |
| `proteus-lib/src/playback/player/effects.rs`                    | Add runtime gating setters and `effect_levels()` accessor      |
| `proteus-lib/src/playback/engine/mod.rs`                        | Thread-plumb shared effect-meter state into the mix engine     |
| `proteus-lib/src/playback/engine/mix/types.rs`                  | Extend `MixThreadArgs` with shared effect-meter handles/config |
| `proteus-lib/src/playback/engine/mix/runner/state.rs`           | Add local metering scratch/state and refresh accounting        |
| `proteus-lib/src/playback/engine/mix/runner/effects_runtime.rs` | Drive metering publication around effect processing            |
| `proteus-lib/src/playback/engine/mix/effects.rs`                | Accept a metering hook/context around each effect boundary     |
| `proteus-lib/Cargo.toml`                                        | Add `effect-meter` feature                                     |
| `proteus-cli/Cargo.toml`                                        | Forward the feature without enabling it by default             |

### Phase 2: Tier 2 analytical curves

**Files to create**

| File                                              | Purpose                                         |
| ------------------------------------------------- | ----------------------------------------------- |
| `proteus-lib/src/dsp/meter/frequency_response.rs` | Pure helpers for analytical response generation |

**Files to modify**

| File                                                 | Change                                                      |
| ---------------------------------------------------- | ----------------------------------------------------------- |
| `proteus-lib/src/dsp/effects/core/biquad.rs`         | Add pure magnitude-response helper for LPF/HPF coefficients |
| `proteus-lib/src/dsp/effects/multiband_eq/biquad.rs` | Add pure response helpers for peaking/shelf sections        |
| `proteus-lib/src/dsp/effects/mod.rs`                 | Add immutable `AudioEffect` dispatch for response queries   |
| `proteus-lib/src/dsp/effects/low_pass.rs`            | Expose analytical curve computation from settings           |
| `proteus-lib/src/dsp/effects/high_pass.rs`           | Expose analytical curve computation from settings           |
| `proteus-lib/src/dsp/effects/multiband_eq/mod.rs`    | Expose per-section and composite response computation       |
| `proteus-lib/src/playback/player/effects.rs`         | Add `effect_frequency_responses()`                          |

### Phase 3: Tier 3 spectral analysis

**Files to create**

| File                                    | Purpose                                                          |
| --------------------------------------- | ---------------------------------------------------------------- |
| `proteus-lib/src/dsp/meter/spectral.rs` | FFT planner wrapper, windowing, bucket reduction, snapshot types |

**Files to modify**

| File                                                            | Change                                                          |
| --------------------------------------------------------------- | --------------------------------------------------------------- |
| `proteus-lib/src/playback/effect_meter.rs`                      | Add runtime spectral configuration and shared snapshot storage  |
| `proteus-lib/src/playback/player/effects.rs`                    | Add spectral enable/refresh setters and `effect_band_levels()`  |
| `proteus-lib/src/playback/engine/mix/runner/state.rs`           | Add per-effect spectral accumulators and lazy FFT state         |
| `proteus-lib/src/playback/engine/mix/runner/effects_runtime.rs` | Feed accumulation buffers and publish decimated spectral frames |
| `proteus-lib/Cargo.toml`                                        | Add `effect-meter-spectral = ["effect-meter", "real-fft"]`      |
| `proteus-cli/Cargo.toml`                                        | Forward `effect-meter-spectral` without enabling it by default  |

---

## Integration Notes

### ST-13 hot-path allocations

This feature should preserve the existing scratch-buffer design introduced by [`ST-13`](./ST-13-effect-chain-hot-path-allocations.md).

Implications:

- read from `scratch_a` / `scratch_b`
- pre-allocate level/spectral scratch storage
- no per-tick heap allocation in the mix loop

### ST-24 effects mutex handoff

[`ST-24`](./ST-24-effects-mutex-handoff.md) is already resolved. The mix thread owns `local_effects` and does not process under the shared effects mutex anymore.

Implication:

- do **not** publish metering data through the shared effects mutex

### FR-02 parameter smoothing

[`FR-02`](./FR-02-inline-effect-parameter-smoothing.md) already introduced coefficient and parameter smoothing.

Implications:

- Tier 1 measures the actual post-fade/post-smoothing audio because it wraps `run_effect_chain()`
- Tier 2 should read from effect **settings**, so the UI curve reflects the target control state during ramps
- Tier 3 measures the actual signal energy after smoothing because it observes the processed audio path

### Effect-chain changes at runtime

When the effect chain changes structurally:

- resize or rebuild all per-effect metering state
- zero newly created snapshots before publishing
- rebuild spectral analyzers only for relevant effect slots

During a full-chain inline transition:

- freeze the last stable Tier 1 / Tier 3 snapshot
- resume publication after the new chain becomes active

---

## Optimization Notes

- **No hidden always-on work.** If a runtime gate is off, that tier should be cold.
- **Refresh by sample frames, not chunk count.** Chunk sizes already vary.
- **Do not block the mix thread.** Use `try_lock()` or an atomic handoff; never wait for the reader.
- **Pre-allocate hot-path buffers.** Resize on chain or channel-count changes, not per publication.
- **Tier 2 should stay pure.** Keep it out of the mix thread and compute from settings on demand.
- **Spectral state can be lazy.** Create FFT plans and analysis buffers when the tier is first enabled.

---

## Acceptance Criteria

### Tier 1

- [x] `effect-meter` is off by default
- [x] public metering types and `Player` methods remain available when the feature is off
- [x] `effect_levels()` returns `None` when the feature is not compiled or runtime metering is disabled
- [x] runtime level metering is disabled by default
- [x] enabling runtime level metering adds no steady-state allocations and no blocking on the mix thread
- [x] refresh cadence is based on sample frames, not chunk count
- [x] enable-fade output metering reflects the crossfaded signal actually heard
- [x] full-chain inline transitions freeze the last stable snapshot instead of publishing ambiguous data
- [x] unit tests verify `measure_peak_rms()` with known signals
- [x] existing playback tests still pass and metering does not alter audio output

### Tier 2

- [x] analytical curves are available for LPF, HPF, and multiband EQ
- [x] multiband EQ exposes both composite and per-section curves
- [x] response computation reads from effect settings, not mutable DSP state
- [x] on-demand queries add no mix-thread work
- [x] unit tests verify known response points (for example LPF at cutoff is near -3 dB for Butterworth-like settings)

### Tier 3

- [x] `effect-meter-spectral` is off by default
- [x] runtime spectral analysis is disabled by default
- [x] when runtime spectral analysis is off, no spectral accumulation or FFT work is performed
- [x] low-pass and high-pass spectral buckets split at cutoff frequency
- [x] multiband EQ spectral buckets are documented as control-aligned analysis buckets, not exact per-filter isolation
- [x] full-chain inline transitions freeze the last stable spectral snapshot
- [x] FFT plans are reused across refreshes
- [x] unit tests verify bucket energy with known single-tone inputs

---

## Status

Done.
