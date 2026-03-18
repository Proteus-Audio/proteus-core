# FR-02: Inline Effect Parameter Smoothing for DAW-Style Real-Time Editing

## Summary

Individual effect parameter changes (gain, EQ frequency/Q, compressor threshold, filter cutoff, etc.) currently take effect instantaneously at the next chunk boundary with no smoothing or ramping. This produces audible discontinuities (clicks, pops, zipper noise) when parameters are adjusted during live playback in a DAW-style authoring workflow. Only full chain replacements via `set_effects_inline` get crossfaded; per-parameter tweaks have no transition at all.

---

## Motivation

The library backs an authoring application where users adjust individual effect knobs while listening. Today, the two mechanisms for live effect updates each have trade-offs that make fine-grained DAW-style parameter editing suboptimal:

1. **`EffectSettingsCommand` queue** (reverb enable/mix only) — instantaneous per-field writes on the mix thread's local copy. No smoothing. Limited to two reverb-specific commands.

2. **`set_effects_inline`** (full chain swap with crossfade) — works for replacing the entire chain but is heavyweight for a single parameter change: it clones the full chain, warms up all effects, and runs two parallel chains during the crossfade window.

Neither path addresses the core need: smoothly transitioning a single parameter (e.g., gain from 0.8 to 1.2) across a short ramp so the change is inaudible beyond the intended effect.

### What goes wrong today

| Parameter change | Artifact | Root cause |
|---|---|---|
| Gain adjustment | Click/pop at chunk boundary | Instantaneous multiplicative discontinuity |
| LPF/HPF cutoff sweep | Click, possible instability | Biquad coefficients replaced mid-stream; filter history reflects old coefficients |
| Compressor threshold | Gain jump | `ensure_state` detects parameter mismatch → creates fresh `CompressorState` with `current_gain_db = 0.0` |
| EQ band gain/Q | Click, transient ring | Biquad state reset on coefficient change |
| Reverb mix | Zipper noise on fast sweeps | No per-sample interpolation between old and new mix values |

---

## Current architecture (for context)

### How parameters reach the mix thread

```
Control thread                           Mix thread
──────────────                           ──────────

set_reverb_mix(0.7)
├─ push EffectSettingsCommand::SetReverbMix(0.7)
└─ write shared effects (for UI reads)
                                         ┌─ chunk boundary ──────────────┐
                                         │ drain_effect_settings_commands │
                                         │ → local_effects[i].mix = 0.7  │
                                         │ process_effects(samples)       │
                                         │ → instant discontinuity        │
                                         └────────────────────────────────┘

set_effects_inline(new_chain)
├─ enqueue InlineEffectsUpdate
                                         ┌─ chunk boundary ──────────────┐
                                         │ apply_effect_runtime_updates   │
                                         │ → crossfade old/new chains     │
                                         │ → runs BOTH chains per chunk   │
                                         │ → 2x DSP cost during transition│
                                         └────────────────────────────────┘
```

### Key files

| File | Relevance |
|---|---|
| `playback/player/effects.rs` | Public API: `set_reverb_mix`, `set_effects_inline`, `set_effects` |
| `playback/engine/mix/runner/effects_runtime.rs` | Mix-thread consumption: `drain_effect_settings_commands`, `process_effects`, crossfade logic |
| `playback/engine/mix/types.rs` | `EffectSettingsCommand` enum (currently only `SetReverbEnabled`, `SetReverbMix`) |
| `dsp/effects/core/biquad.rs` | `BiquadState` — reconstructed from scratch when parameters change |
| `dsp/effects/compressor.rs` | `CompressorState` — reset to `current_gain_db = 0.0` on any parameter delta |
| `dsp/effects/gain.rs` | Reads `self.settings.gain` directly per-chunk, no smoothing |
| `dsp/effects/low_pass.rs` / `high_pass.rs` | `ensure_state` — full biquad reconstruction on any freq/Q change |

---

## Design

### Principle: per-parameter ramps, not chain-level crossfades

For individual knob turns, the right solution is parameter-level smoothing inside each effect, not running two chains in parallel. This is both cheaper (no doubled DSP) and more correct (the effect's internal state remains coherent throughout the transition).

### A. Generalise the `EffectSettingsCommand` queue

Currently the command queue is limited to two reverb-specific variants. Extend it to support any effect parameter change:

```rust
pub enum EffectSettingsCommand {
    // Existing (keep for backwards compat)
    SetReverbEnabled(bool),
    SetReverbMix(f32),

    // New: targeted parameter update for any effect at a chain index
    SetEffectParameter {
        /// Index into the effect chain
        effect_index: usize,
        /// Which parameter to update
        parameter: EffectParameter,
    },
    /// Toggle enabled state for any effect by chain index
    SetEffectEnabled {
        effect_index: usize,
        enabled: bool,
    },
}

/// Identifies a specific parameter on an effect.
pub enum EffectParameter {
    Gain(f32),
    LowPassFreqHz(u32),
    LowPassQ(f32),
    HighPassFreqHz(u32),
    HighPassQ(f32),
    CompressorThresholdDb(f32),
    CompressorRatio(f32),
    CompressorAttackMs(f32),
    CompressorReleaseMs(f32),
    CompressorMakeupDb(f32),
    ReverbMix(f32),
    ReverbEnabled(bool),
    Pan(f32),
    // ... extend as needed per effect type
}
```

Add corresponding `Player` methods:

```rust
impl Player {
    /// Update a single parameter on the effect at `index` in the chain.
    ///
    /// The update is queued for the mix thread and also applied to the shared
    /// chain for UI reads. Returns `false` if `index` is out of range.
    pub fn set_effect_parameter(&self, index: usize, param: EffectParameter) -> bool { ... }

    /// Toggle enabled/disabled for the effect at `index`.
    pub fn set_effect_enabled(&self, index: usize, enabled: bool) -> bool { ... }
}
```

This gives the authoring app a lightweight path for every knob, not just reverb mix.

### B. Per-parameter smoothing inside effects

The core change: effects that are sensitive to parameter discontinuities should internally ramp between old and new parameter values over a short window (~5-10 ms). This is standard practice in audio plugin development.

#### B.1 Gain smoothing

The simplest and highest-impact case. Track a `smoothed_gain` value alongside the target:

```rust
struct GainState {
    current: f32,
    target: f32,
    ramp_samples_remaining: usize,
    ramp_increment: f32,
}
```

On each sample, if `current != target`: advance `current` by `ramp_increment`. This is a single addition per sample — negligible cost. When a new target arrives, compute `ramp_increment = (new_target - current) / ramp_samples` where `ramp_samples` is derived from a fixed ramp time (e.g., 5 ms at the current sample rate).

This pattern also applies to reverb `mix`/`dry_wet` parameters and the `pan` effect.

#### B.2 Biquad coefficient interpolation (LPF, HPF, EQ)

Biquad filters are particularly sensitive to coefficient changes because the filter's internal history (delay line) is coupled to the old coefficients. Replacing coefficients mid-stream creates a transient that depends on the current state of the delay line.

Two approaches, in order of recommendation:

**Option 1 — Coefficient smoothing (preferred):**

Instead of reconstructing `BiquadState` when parameters change, compute the *target* coefficients and ramp each coefficient toward the target over a short window. The filter continues to process with coefficients that gradually transition:

```rust
struct SmoothedBiquadState {
    current: BiquadCoefficients,
    target: BiquadCoefficients,
    ramp_remaining: usize,
    // per-coefficient deltas
    d_b0: f32, d_b1: f32, d_b2: f32, d_a1: f32, d_a2: f32,
    // ... plus existing delay line state
}
```

On each sample, if ramping: `current.b0 += d_b0`, etc. This preserves the delay line continuity — no click, no transient, no state reset. Cost: 5 additions per sample during the ramp window only.

**Option 2 — Crossfade between two filter instances:**

Run the old biquad and a new biquad in parallel for the ramp duration, crossfading their outputs. More expensive (2x filter cost during ramp) but simpler to implement and guaranteed artifact-free.

Recommend Option 1 for LPF/HPF (single biquad) and Option 2 for multiband EQ (multiple biquads — coefficient smoothing across N bands is more complex to manage correctly).

#### B.3 Compressor state continuity

The current `ensure_state` pattern detects *any* parameter change and rebuilds `CompressorState` from scratch, resetting `current_gain_db` to 0.0. This causes a gain jump.

Fix: separate parameters that can be updated in-place (threshold, ratio, makeup gain) from those that require state recomputation (attack/release time constants):

```rust
impl CompressorState {
    /// Update parameters that can change without resetting the gain envelope.
    fn update_parameters(&mut self, params: &CompressorParams) {
        self.threshold_db = params.threshold_db;
        self.ratio = params.ratio;
        self.makeup_gain_db = params.makeup_gain_db;
        // Recompute time constants but keep current_gain_db intact
        self.attack_coeff = time_to_coeff(params.attack_ms, self.sample_rate);
        self.release_coeff = time_to_coeff(params.release_ms, self.sample_rate);
    }
}
```

The key insight: `current_gain_db` represents the compressor's envelope follower state — it should *never* be reset when parameters change during playback. The compressor naturally tracks toward the new gain target through its attack/release ballistics. Only `sample_rate` and `channels` changes (which don't happen during playback) truly require a full state reset.

#### B.4 Enable/disable crossfade

Toggling `enabled` currently switches between full processing and passthrough at a chunk boundary, creating a discontinuity equal to the difference between wet and dry signals. Add a short fade when `enabled` changes:

```rust
// In the DspEffect trait or in the AudioEffect dispatch
fn process_with_enable_fade(
    &mut self,
    enabled_changed: bool,
    samples: &[f32],
    context: &EffectContext,
) -> ... {
    if enabled_changed {
        // Run both paths, crossfade over ~256 samples
        let dry = samples;
        let wet = self.process(samples, context, false);
        // fade from old to new
    }
}
```

This could be handled at the `AudioEffect` enum level rather than inside each effect, keeping individual effects simple.

### C. Ramp time configuration

Add a configurable ramp duration to `PlaybackBufferSettings`:

```rust
pub struct PlaybackBufferSettings {
    // ... existing fields ...
    /// Duration in ms for parameter smoothing ramps (default: 5.0).
    pub parameter_ramp_ms: f32,
}
```

Effects read this via `EffectContext` (add a `parameter_ramp_ms` field or `parameter_ramp_samples` derived from sample rate). 5 ms is a good default: short enough to feel instantaneous, long enough to eliminate clicks at all sample rates (240 samples at 48 kHz).

---

## Implementation plan

### Phase 1: Infrastructure + gain/mix smoothing

The highest-impact, lowest-risk changes.

**Files to create:**

| File | Purpose |
|---|---|
| `proteus-lib/src/dsp/effects/core/smoother.rs` | `ParamSmoother` — reusable linear ramp primitive |

**Files to modify:**

| File | Change |
|---|---|
| `playback/engine/mix/types.rs` | Extend `EffectSettingsCommand` with `SetEffectParameter`, `SetEffectEnabled` |
| `playback/player/effects.rs` | Add `set_effect_parameter()`, `set_effect_enabled()` |
| `playback/engine/mix/runner/effects_runtime.rs` | Handle new command variants in `drain_effect_settings_commands` |
| `dsp/effects/gain.rs` | Add `ParamSmoother` for gain value; ramp in `process_into` |
| `dsp/effects/pan.rs` | Add `ParamSmoother` for pan value |
| `dsp/effects/core/mod.rs` | `pub(crate) mod smoother;` |
| `dsp/effects/mod.rs` | Expose ramp duration through `EffectContext` |
| `playback/player/settings.rs` | Add `parameter_ramp_ms` to `PlaybackBufferSettings` |

**`ParamSmoother` design:**

```rust
/// Zero-allocation linear parameter ramp.
pub(crate) struct ParamSmoother {
    current: f32,
    target: f32,
    increment: f32,
    remaining: usize,
}

impl ParamSmoother {
    pub fn new(initial: f32) -> Self { ... }
    pub fn set_target(&mut self, target: f32, ramp_samples: usize) { ... }
    pub fn next(&mut self) -> f32 { ... }
    pub fn is_settled(&self) -> bool { ... }
    pub fn current(&self) -> f32 { ... }
}
```

Cost: one `f32` addition and one `usize` decrement per sample during ramp. Zero cost when settled.

### Phase 2: Biquad coefficient smoothing

**Files to modify:**

| File | Change |
|---|---|
| `dsp/effects/core/biquad.rs` | Add `SmoothedBiquadState` that ramps coefficients; keep `BiquadState` for non-smoothed use |
| `dsp/effects/low_pass.rs` | Use `SmoothedBiquadState`; `ensure_state` updates target coefficients instead of reconstructing |
| `dsp/effects/high_pass.rs` | Same as low_pass |
| `dsp/effects/multiband_eq/biquad.rs` | Use crossfade approach for multi-band (dual instances during transition) |

### Phase 3: Compressor continuity + enable/disable fade

**Files to modify:**

| File | Change |
|---|---|
| `dsp/effects/compressor.rs` | Split `ensure_state` into `update_parameters` (preserves gain envelope) and `rebuild_state` (only for sample_rate/channels changes) |
| `dsp/effects/mod.rs` | Add enable/disable crossfade at the `AudioEffect` dispatch level |
| `playback/engine/mix/runner/effects_runtime.rs` | Track previous enabled state per effect for fade detection |

---

## Interaction with existing systems

### Relationship to `set_effects_inline` crossfade

Parameter smoothing and chain-level crossfade serve different purposes:

- **Parameter smoothing** (this ticket): single knob turns during live editing. Cheap, per-parameter, preserves effect state.
- **Chain crossfade** (`set_effects_inline`): replacing the entire chain structure (adding/removing/reordering effects). More expensive, but necessary when the chain topology changes.

Both mechanisms should coexist. The authoring app should use `set_effect_parameter()` for knob turns and `set_effects_inline()` for structural chain changes (add/remove effect, reorder chain).

### Relationship to FR-01 (per-effect metering)

No conflict for Tier 1 (time-domain levels) or Tier 3 (spectral analysis) — metering observes the output of `process_into` which already reflects smoothed parameters. The metering snapshots will naturally show the smooth transition rather than a discontinuous jump.

**Tier 2 interaction (analytical filter response curves).** FR-01 Tier 2 computes analytical frequency response from biquad coefficients. When Phase 2 of this ticket introduces coefficient smoothing, mid-ramp coefficients don't represent a coherent filter shape. `SmoothedBiquadState` (or equivalent) must expose a `target_coefficients()` accessor so FR-01's `frequency_response()` evaluates against the destination shape, not the in-progress ramp values.

**Enable/disable crossfade placement.** Phase 3's enable/disable crossfade should happen inside the `AudioEffect` dispatch (within `process_into`), not at the `run_effect_chain` level. This keeps it inside FR-01's metering measurement brackets so Tier 1 meters naturally capture the post-fade output without additional coordination.

### Real-time safety

All smoothing state lives on the mix thread's local copy. No new locks, no new allocations in steady state. `ParamSmoother` is 4 fields (16 bytes). Coefficient smoothing adds 5 floats (20 bytes) per biquad. Well within real-time constraints.

---

## Acceptance criteria

### Phase 1
- [x] `ParamSmoother` primitive exists with tests for ramp accuracy and settling
- [x] Gain effect uses `ParamSmoother`; sweeping gain produces no clicks on a sine wave test signal
- [x] Pan effect uses `ParamSmoother`
- [x] Reverb mix parameters use `ParamSmoother` (all three reverb variants)
- [x] `set_effect_parameter()` API works for gain, pan, and reverb mix
- [x] `parameter_ramp_ms` is configurable via `PlaybackBufferSettings`
- [x] No new allocations in the steady-state mix loop
- [x] Existing tests pass

### Phase 2
- [x] LPF/HPF cutoff and Q sweeps produce no clicks on test signals
- [x] Biquad delay line state is preserved across coefficient changes
- [x] Multiband EQ band adjustments produce no clicks
- [x] Filter response remains stable during fast parameter sweeps

### Phase 3
- [x] Compressor threshold/ratio changes do not reset gain envelope
- [x] Compressor attack/release changes recompute coefficients without gain jump
- [x] Toggling any effect's `enabled` flag crossfades over ~5 ms
- [x] Unit tests verify enable/disable fade produces no discontinuity above threshold

---

## Status

Complete.
