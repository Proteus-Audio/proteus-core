# SI-04: DSP Effects — Oversized Files

## Files affected

| File | Lines |
|---|---|
| `proteus-lib/src/dsp/effects/multiband_eq.rs` | 761 |
| `proteus-lib/src/dsp/effects/diffusion_reverb/mod.rs` | 740 |
| `proteus-lib/src/dsp/effects/convolution_reverb/mod.rs` | 621 |

---

## `multiband_eq.rs` (761 lines)

### Current structure

The file mixes two distinct layers:

1. **Effect layer** — `EqPointSettings`, `LowEdgeFilterSettings`, `HighEdgeFilterSettings`,
   `MultibandEqSettings`, `MultibandEqEffect`, `MultibandEqState`, `EqPointParams`,
   `LowEdgeParams`, `HighEdgeParams` + the `DspEffect` impl (lines 12–365)
2. **DSP primitive layer** — `BiquadCoefficients`, `BiquadDesign`, `Biquad`, and all
   coefficient computation functions (`coefficients`, `peaking_coefficients`,
   `low_pass_coefficients`, `high_pass_coefficients`, `low_shelf_coefficients`,
   `high_shelf_coefficients`, `normalized_coefficients`) (lines 366–620)
3. Sanitize helpers and tests (lines 565–761)

The biquad primitives are generic DSP infrastructure that could be reused by other
effects. They have no dependency on the EQ-specific settings types.

### Proposed split

Convert to a directory:

```
dsp/effects/multiband_eq/
├── mod.rs       # Re-exports, MultibandEqEffect/Settings/State, DspEffect impl — ~350 lines
└── biquad.rs    # BiquadCoefficients, BiquadDesign, Biquad, all coefficient
                 # functions, sanitize_freq — ~250 lines
```

If the biquad type proves useful to other effects in the future, it can be promoted to
`dsp/effects/core/biquad.rs`.

**Expected result**: `mod.rs` ≤350 lines, `biquad.rs` ~250 lines.

---

## `diffusion_reverb/mod.rs` (740 lines)

### Current structure

The diffusion reverb is already a directory but the entire implementation lives in
`mod.rs`. The file contains:

1. **Effect layer** — constants, `DiffusionReverbSettings`, `DiffusionReverbEffect`,
   `Tuning`, `DiffusionReverbState` + `DspEffect` impl (lines 24–461)
2. **DSP primitive layer** — `ReverbLane`, `DelayLine`, `CombFilter`, `AllpassFilter`,
   `OnePoleLowpass`, and their impls (lines 462–696)
3. Helpers and tests (lines 688–740)

The primitive types (`DelayLine`, `CombFilter`, `AllpassFilter`, `OnePoleLowpass`,
`ReverbLane`) are pure DSP building blocks with no dependency on the effect settings.

### Proposed split

```
dsp/effects/diffusion_reverb/
├── mod.rs        # Effect layer: settings, effect struct, DspEffect impl,
│                 # Tuning, DiffusionReverbState — ~350 lines
└── primitives.rs # ReverbLane, DelayLine, CombFilter, AllpassFilter,
                  # OnePoleLowpass, delay_samples — ~300 lines
```

**Expected result**: `mod.rs` ≤350 lines, `primitives.rs` ~300 lines.

---

## `convolution_reverb/mod.rs` (621 lines)

### Current structure

The convolution reverb directory already has `reverb.rs`, `impulse_response.rs`, and
`convolution.rs`. However `mod.rs` itself is 621 lines and contains:

1. **Settings and types** — `ConvolutionReverbSettings`, `ConvolutionReverbEffect`,
   `ImpulseResponseSpec`, related enums (lines 1–200)
2. **IR loading logic** — `load_ir_from_spec`, helper functions for embedded and
   file-based IR resolution (lines 200–500)
3. **`DspEffect` impl** — `process`, `reset_state` (lines 500–580)
4. Tests (lines 580–621)

The IR loading logic is substantial and independent of the effect's DSP processing.

### Proposed split

```
dsp/effects/convolution_reverb/
├── mod.rs              # Settings, effect struct, DspEffect impl — ~250 lines
├── ir_loader.rs        # load_ir_from_spec, embedded/file IR helpers — ~280 lines
├── impulse_response.rs
├── reverb.rs
└── convolution.rs
```

**Expected result**: `mod.rs` ≤250 lines, `ir_loader.rs` ~280 lines.

---

## Acceptance criteria

- [ ] All existing tests pass (`cargo test -p proteus-lib --all-features`)
- [ ] `cargo check --all-features` shows no new errors or warnings
- [ ] Each new file is ≤400 lines
- [ ] Public re-exports in each `mod.rs` preserve the import paths used by callers
  (particularly `ImpulseResponseSpec`, `ConvolutionReverbSettings`, etc.)
