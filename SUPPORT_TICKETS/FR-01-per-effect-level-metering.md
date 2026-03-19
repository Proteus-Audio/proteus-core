# FR-01: Per-Effect Input/Output Level Metering and Spectral Analysis

## Summary

Add optional per-effect input and output level metering to the DSP chain, along with per-band spectral analysis for frequency-shaping effects (multiband EQ, lowpass, highpass). This enables GUI applications to display real-time visual feedback for each effect — input/output bars, gain reduction indicators, and frequency response curves — without penalizing headless or CLI consumers.

---

## Motivation

Currently the only metering in the system is the `OutputMeter` at the final output stage (`playback/output_meter.rs`). There is no way for a consumer to inspect signal levels at each point in the DSP chain. GUI applications need:

- **Per-effect I/O levels**: input and output peak/RMS for every effect in the chain, enabling visual gain staging feedback
- **Per-band levels for EQ/filter effects**: spectral energy breakdown by configured EQ band, so a GUI can visualize the frequency-domain impact of each filter point
- **Filter response curves**: the magnitude response of biquad-based effects (multiband EQ, lowpass, highpass), enabling overlay of the analytical filter shape

---

## Feature gating

Gate all metering behind a new **`effect-meter`** feature flag, following the `output-meter` precedent:

```toml
# proteus-lib/Cargo.toml
[features]
effect-meter = []

# proteus-cli/Cargo.toml
[features]
effect-meter = ["proteus-lib/effect-meter"]
```

When disabled, all metering types compile to zero-cost no-ops. The feature is **off by default** — only applications that need it (GUI frontends) pay the cost.

---

## Design: three tiers

Split the work into three tiers of increasing computational cost. Each tier builds on the previous one but can be implemented independently. All three tiers live behind the single `effect-meter` feature flag, with the expensive tier (spectral FFT) additionally gated by a runtime configuration flag so consumers can enable/disable it dynamically.

### Tier 1 — Time-domain peak/RMS levels (near-zero cost)

**What it provides:** Per-effect input peak, input RMS, output peak, and output RMS, per channel.

**Where it hooks in:** `run_effect_chain()` in `playback/engine/mix/effects.rs`. This is the only call site — individual effects do not need modification.

**How it works:**

```
for each effect in chain:
    if metering_due:
        measure scratch_a → input_levels[effect_index]
    effect.process_into(scratch_a, scratch_b, ...)
    swap(scratch_a, scratch_b)
    if metering_due:
        measure scratch_a → output_levels[effect_index]
```

The measurement pass iterates the interleaved samples once, tracking per-channel peak (`abs().max()`) and sum-of-squares for RMS. This is the same work the compressor already does internally for its own gain computation — it's essentially free relative to the DSP processing itself.

**Decimation:** Metering does not need to run on every chunk. Add a frame counter and only measure every `N` chunks, where `N` is derived from a configurable refresh rate (e.g., 30 Hz at 48 kHz / 1024-sample chunks ≈ every ~1.5 chunks). This keeps the overhead proportional to the display refresh rate, not the audio sample rate.

**Data structure:**

```rust
/// Per-channel peak and RMS snapshot for one measurement point.
#[derive(Debug, Clone, Default)]
pub struct LevelSnapshot {
    /// Per-channel peak values (linear).
    pub peak: Vec<f32>,
    /// Per-channel RMS values (linear).
    pub rms: Vec<f32>,
}

/// Input and output levels for a single effect.
#[derive(Debug, Clone, Default)]
pub struct EffectLevelSnapshot {
    pub input: LevelSnapshot,
    pub output: LevelSnapshot,
}
```

**Exposing to consumers:** Store snapshots in a pre-allocated `Vec<EffectLevelSnapshot>` inside `MixLoopState`. After each metering pass, publish the snapshot to a shared structure readable by the player API thread — either via an `Arc<Mutex<_>>` (matching the existing effects pattern) or, preferably, a lock-free single-producer/single-consumer mechanism like a `std::sync::atomic`-backed swap buffer to avoid any lock contention on the mix thread.

A simple approach that avoids new dependencies: use a double-buffer behind an `AtomicBool` flag. The mix thread writes to the back buffer and flips the flag; the reader thread reads from the front buffer. This is the same pattern used by many real-time audio metering systems.

**Player API surface:**

```rust
impl Player {
    /// Returns the most recent per-effect level snapshots, or an empty vec
    /// if `effect-meter` is disabled or no data is available yet.
    pub fn effect_levels(&self) -> Vec<EffectLevelSnapshot> { ... }
}
```

**Estimated cost:** ~1 `abs()` + 1 multiply + 2 comparisons per sample per metered boundary, only at display refresh rate. Negligible.

---

### Tier 2 — Analytical filter response curves (cheap, no FFT)

**What it provides:** The magnitude response of biquad-based effects (multiband EQ, lowpass, highpass) as a set of (frequency, gain_dB) points. This is the classic EQ curve overlay in DAW plugin UIs.

**Where it hooks in:** Each biquad-based effect exposes its current filter coefficients. A new method on the relevant effects computes the analytical frequency response.

**How it works:**

Biquad transfer function magnitude at frequency `f` given coefficients `(b0, b1, b2, a0, a1, a2)`:

```
H(e^{jω}) where ω = 2π * f / sample_rate

|H(ω)|² = (b0² + b1² + b2² + 2(b0*b1 + b1*b2)*cos(ω) + 2*b0*b2*cos(2ω))
         / (a0² + a1² + a2² + 2(a0*a1 + a1*a2)*cos(ω) + 2*a0*a2*cos(2ω))
```

Evaluate at N log-spaced frequency points (e.g., 128 points from 20 Hz to 20 kHz). For the multiband EQ, multiply the per-band responses to get the composite curve. This is pure arithmetic — no signal processing, no FFT.

**Data structure:**

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

**Where to add the method:**

Add a trait method with a default no-op to `DspEffect`:

```rust
/// Return the analytical frequency response curve, if applicable.
///
/// Only meaningful for biquad-based filter effects. The default
/// returns `None`.
fn frequency_response(&self, _sample_rate: u32, _num_points: usize) -> Option<FilterResponseCurve> {
    None
}
```

Override in `MultibandEqEffect`, `LowPassFilterEffect`, and `HighPassFilterEffect`. Dispatch through `AudioEffect` to the player API.

**When to compute:** Only on demand (when the player API method is called), not on every process cycle. The coefficients change rarely (only when settings change), so the curve can be cached and invalidated on settings mutation. This makes the cost essentially zero during steady-state playback.

**Player API surface:**

```rust
impl Player {
    /// Returns analytical frequency response curves for each filter-type
    /// effect in the chain. Non-filter effects return `None` in their slot.
    pub fn effect_frequency_responses(&self) -> Vec<Option<FilterResponseCurve>> { ... }
}
```

**Estimated cost:** ~128 * num_bands * 10 floating-point ops, only when the UI requests it and settings have changed. Negligible.

---

### Tier 3 — Spectral analysis via FFT (moderate cost, runtime-gated)

**What it provides:** Per-band spectral energy levels for the input and output of frequency-shaping effects, enabling a GUI to show "how much energy is in each EQ band" as animated bars.

**Why FFT is needed:** Tier 1 gives broadband levels; Tier 2 gives the filter's theoretical shape. But to show *actual signal energy per frequency band*, you need to decompose the signal into the frequency domain.

**Runtime gating:** Even within the `effect-meter` feature flag, spectral analysis should be toggled via a runtime setting (default off). This lets a GUI app enable it only when the EQ editor panel is open:

```rust
impl Player {
    /// Enable or disable per-band spectral analysis for filter effects.
    /// When disabled, `effect_band_levels()` returns empty results.
    pub fn set_spectral_analysis_enabled(&self, enabled: bool) { ... }
}
```

**How it works:**

1. **Accumulate a window of input samples** before each filter-type effect in the chain. Use a ring buffer per metering point (pre-allocated at engine startup), sized to the FFT window (e.g., 2048 samples per channel).

2. **At display refresh rate** (not every chunk), run a windowed FFT:
   - Apply a Hann window to the accumulated samples
   - Compute the real FFT using the existing `realfft` crate (already a dependency for convolution reverb)
   - Partition FFT bins into the configured EQ band ranges
   - Sum magnitudes per band to produce per-band energy levels

3. **Repeat for the output** of the same effect to show the "after" state.

4. **Publish** per-band input/output levels via the same double-buffer mechanism as Tier 1.

**FFT reuse:** The `realfft` crate is already available behind the `real-fft` feature (on by default). The `effect-meter` feature should depend on `real-fft` for this tier, or the spectral tier could be a sub-feature (`effect-meter-spectral = ["effect-meter", "real-fft"]`). Recommended: keep it simple — `effect-meter` implies `real-fft` availability since it's already the default.

**Band partitioning for multiband EQ:**

Map FFT bins to EQ bands using the configured `EqPointSettings` frequencies as crossover points. For `N` EQ points at frequencies `f1, f2, ..., fN`, create `N+1` bands:

```
Band 0: [0, f1)
Band 1: [f1, f2)
...
Band N: [fN, Nyquist]
```

If edge filters are configured (lowpass/highpass), use their cutoff frequencies as additional band boundaries.

**Data structure:**

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

**Player API surface:**

```rust
impl Player {
    /// Returns per-band spectral levels for each filter-type effect.
    /// Returns empty results for non-filter effects or when spectral
    /// analysis is disabled.
    pub fn effect_band_levels(&self) -> Vec<Option<EffectBandSnapshot>> { ... }
}
```

**Estimated cost:**

At 48 kHz stereo with a 2048-point FFT at 30 Hz refresh:
- FFT: ~2048 * log2(2048) ≈ 22,528 multiplies per transform, twice per metering point (input + output), per filter effect
- At 30 Hz refresh with 2 filter effects: ~2.7M multiplies/second
- Modern CPUs handle this comfortably, but it's non-trivial — hence the runtime gate

For comparison, the convolution reverb already runs continuous FFTs on every chunk, so this is far less than what the system already supports under the `real-fft` feature.

---

## Implementation plan

### Phase 1: Infrastructure and Tier 1

**Files to create:**

| File | Purpose |
|---|---|
| `proteus-lib/src/dsp/meter/mod.rs` | `LevelSnapshot`, `EffectLevelSnapshot`, measurement helpers |
| `proteus-lib/src/dsp/meter/level.rs` | `measure_peak_rms(samples, channels) -> LevelSnapshot` |

**Files to modify:**

| File | Change |
|---|---|
| `proteus-lib/src/dsp/mod.rs` | Add `pub mod meter;` (gated on `effect-meter`) |
| `proteus-lib/src/playback/engine/mix/effects.rs` | Add metering capture around each effect in `run_effect_chain` |
| `proteus-lib/src/playback/engine/mix/runner/state.rs` | Add `Vec<EffectLevelSnapshot>` and frame counter to `MixLoopState` |
| `proteus-lib/src/playback/player/mod.rs` | Add `effect_levels()` public accessor |
| `proteus-lib/src/playback/player/effects.rs` | Wire metering state through player shared state |
| `proteus-lib/Cargo.toml` | Add `effect-meter = []` feature |
| `proteus-cli/Cargo.toml` | Add `effect-meter = ["proteus-lib/effect-meter"]` feature |

**Key decision — transport mechanism for metering data:**

The mix thread must not block on publishing metering data. Two options:

1. **Double-buffer with `AtomicBool`** (no new dependencies): Pre-allocate two `Vec<EffectLevelSnapshot>`s. Mix thread writes to back buffer, flips atomic flag. Reader reads front buffer. Simple, zero-allocation steady state, no lock.

2. **Reuse existing effects `Mutex` pattern**: Wrap metering data in the same `Arc<Mutex<_>>` that already guards the effects chain. The lock is already held during the chain run, so writing metering data adds no new contention. Reader contention is bounded by display refresh rate.

Option 2 is simpler and the contention is acceptable for metering (dropped frames are fine — the UI just shows the last available data). **Recommend option 2 for initial implementation**, with a note that a lock-free upgrade is possible if profiling reveals contention.

### Phase 2: Tier 2 — Analytical response curves

**Files to create:**

| File | Purpose |
|---|---|
| `proteus-lib/src/dsp/meter/frequency_response.rs` | `FilterResponseCurve`, analytical biquad magnitude computation |

**Files to modify:**

| File | Change |
|---|---|
| `proteus-lib/src/dsp/effects/core/mod.rs` | Add `frequency_response()` default method to `DspEffect` |
| `proteus-lib/src/dsp/effects/core/biquad.rs` | Add `magnitude_at(freq, sample_rate) -> f32` to `BiquadState` |
| `proteus-lib/src/dsp/effects/multiband_eq/mod.rs` | Override `frequency_response()` — compute per-band and composite |
| `proteus-lib/src/dsp/effects/multiband_eq/biquad.rs` | Expose coefficient access for analytical evaluation |
| `proteus-lib/src/dsp/effects/low_pass.rs` | Override `frequency_response()` |
| `proteus-lib/src/dsp/effects/high_pass.rs` | Override `frequency_response()` |
| `proteus-lib/src/dsp/effects/mod.rs` | Dispatch `frequency_response()` through `AudioEffect` enum |
| `proteus-lib/src/playback/player/effects.rs` | Add `effect_frequency_responses()` accessor |

### Phase 3: Tier 3 — Spectral band analysis

**Files to create:**

| File | Purpose |
|---|---|
| `proteus-lib/src/dsp/meter/spectral.rs` | FFT-based spectral analysis, band partitioning, `EffectBandSnapshot` |
| `proteus-lib/src/dsp/meter/window.rs` | Hann window generation and sample accumulation ring buffer |

**Files to modify:**

| File | Change |
|---|---|
| `proteus-lib/src/playback/engine/mix/effects.rs` | Add sample accumulation for spectral points, trigger FFT at refresh rate |
| `proteus-lib/src/playback/engine/mix/runner/state.rs` | Add spectral ring buffers and FFT planner to `MixLoopState` |
| `proteus-lib/src/playback/player/effects.rs` | Add `effect_band_levels()` and `set_spectral_analysis_enabled()` |
| `proteus-lib/Cargo.toml` | `effect-meter` should include `realfft` dependency (or make spectral a sub-feature) |

---

## Integration with existing systems

### Interaction with SI-13 (effect chain allocations)

The `run_effect_chain` ping-pong scratch buffer architecture is ideal for metering — `scratch_a` holds the input before each effect, `scratch_b` receives the output. Metering reads these buffers without modifying them. No new allocations needed in steady state (pre-allocate level accumulators at engine startup).

### Interaction with SI-24 (effects mutex handoff)

Metering data publication happens while the effects mutex is already held (during `run_effect_chain`). If SI-24's recommendations are implemented (snapshot-based handoff), metering data should travel with the snapshot rather than requiring its own synchronization.

### Interaction with FR-02 (inline parameter smoothing)

FR-02 introduces per-parameter smoothing (gain ramps, biquad coefficient interpolation) inside individual effects. Two interactions to be aware of:

**Tier 2 analytical curves and mid-ramp coefficients.** When FR-02's biquad coefficient smoothing is active, the filter's *current* coefficients are mid-ramp values that don't represent a coherent filter shape. `frequency_response()` must evaluate against the *target* coefficients, not the in-progress ramp values, so the UI curve shows the destination shape rather than wobbling during a 5 ms transition. If FR-02 uses `SmoothedBiquadState`, expose a `target_coefficients()` accessor and use that in `frequency_response()`.

**Enable/disable crossfade and metering hook point.** FR-02 Phase 3 proposes an enable/disable crossfade at the `AudioEffect` dispatch level (inside `process_into`). This is the correct boundary for FR-01 compatibility: Tier 1 metering wraps `process_into` from outside in `run_effect_chain`, so it naturally captures the post-fade output without additional coordination. If the enable/disable fade is instead lifted to `run_effect_chain` level, it must be placed *inside* the metering measurement brackets so meters reflect what the listener actually hears.

### Effect chain changes at runtime

When effects are added/removed via `set_effects()` or `set_effects_inline()`, the metering `Vec` must be resized to match. Handle this in the same code path that resizes scratch buffers.

---

## Optimization notes

- **Pre-allocate everything** at engine startup or when the effect chain changes. No allocations in the metering hot path.
- **Decimation is critical.** At 48 kHz stereo with 1024-sample chunks, the mix loop runs ~94 times/second. Display refresh is 10–30 Hz. Skip metering on ~70–90% of chunks.
- **Tier 3 FFT planner reuse.** `realfft::RealFftPlanner` caches plans. Create the planner once in `MixLoopState` and reuse across all metering points.
- **Avoid per-sample branching in effects.** Metering wraps effects from outside (`run_effect_chain`), not inside each effect's `process_into`. Individual effects remain unchanged and branch-free.
- **Tier 2 caching.** Analytical curves only change when filter settings change. Cache the result and invalidate on settings mutation (add a generation counter to settings structs, or simply recompute when the player API is called — the cost is so low that caching may be unnecessary).

---

## Acceptance criteria

### Tier 1
- [ ] `effect-meter` feature flag compiles to zero-cost no-ops when disabled
- [ ] Per-effect input/output peak and RMS levels are available through the `Player` API
- [ ] Metering decimation is configurable and defaults to ~30 Hz
- [ ] No new heap allocations in the steady-state mix loop
- [ ] Existing tests pass; metering does not alter audio output
- [ ] Unit tests for `measure_peak_rms` with known signals

### Tier 2
- [ ] `frequency_response()` returns analytical curves for multiband EQ, lowpass, and highpass
- [ ] Composite and per-band curves are available for multiband EQ
- [ ] Curves update when settings change
- [ ] Unit tests verify known biquad responses (e.g., lowpass at cutoff = −3 dB)

### Tier 3
- [ ] Spectral analysis is disabled by default and togglable at runtime
- [ ] Per-band input/output energy levels are available through the `Player` API
- [ ] FFT uses the existing `realfft` crate; planner is shared and reused
- [ ] Band boundaries match the configured EQ point frequencies
- [ ] Spectral analysis adds no overhead when disabled at runtime
- [ ] Unit tests verify band energy with known single-frequency test signals

---

## Status

Open.
