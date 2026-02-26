# Library Onboarding: DSP Effects and Signal Chain

- [Back to Library Onboarding](./index.md)

## Where the Effect Model Lives

Start in [`proteus-lib/src/dsp/effects/mod.rs`](../../../proteus-lib/src/dsp/effects/mod.rs).

This file defines:

- the effect modules
- the `AudioEffect` enum (serialized config + runtime state carrier)
- `EffectContext` (sample rate, channels, container path, IR settings)
- helper accessors for effect-specific manipulation

## How Effects Are Represented

`AudioEffect` is an enum of concrete effect wrappers (gain, filters, reverbs, compressor, limiter, EQ, pan, etc.).

Each variant:

- carries its own settings/state
- can process interleaved sample buffers
- can reset internal state

This design makes it easy to:

- deserialize effect chains from JSON/container settings
- store one heterogeneous chain in `Vec<AudioEffect>`
- run a generic "process chain" loop in playback code

## Runtime Effect Context

`EffectContext` provides runtime-only data that effect instances should not own permanently, such as:

- `sample_rate`
- `channels`
- container path
- convolution impulse response override spec/tail

This keeps effect instances serializable/configurable while still allowing runtime-dependent processing.

## Warm-Up / State / Tail Behavior

Effects are not all stateless.

- Stateless examples: gain, some simple per-sample transforms
- Stateful examples: reverbs, filters, compressor/limiter smoothing, convolution engines

The `AudioEffect` API includes:

- `process(...)`
- `reset_state()`
- `warm_up(...)`

`warm_up(...)` matters most for convolution reverb initialization (IR loading/building) before real audio arrives.

## Where Effects Meet Playback

The playback pipeline owns the effect chain and applies it to mixed buffers before sink append.

Relevant areas:

- [`proteus-lib/src/playback/engine/mix/effects.rs`](../../../proteus-lib/src/playback/engine/mix/effects.rs)
- [`proteus-lib/src/playback/engine/mix/output_stage.rs`](../../../proteus-lib/src/playback/engine/mix/output_stage.rs)
- [`proteus-lib/src/playback/player/effects.rs`](../../../proteus-lib/src/playback/player/effects.rs) (user-facing controls)

## Extending the Effect System (Mental Checklist)

When adding a new effect:

1. Add module implementation under [`proteus-lib/src/dsp/effects/`](../../../proteus-lib/src/dsp/effects/)
2. Export settings/effect type in `dsp/effects/mod.rs`
3. Add `AudioEffect` enum variant with serde rename
4. Wire `process`, `reset_state`, and any accessors
5. Ensure CLI/project-file JSON defaults (if needed) include/understand it
6. Add knowledge-base docs (effect + algorithm if applicable)

## Related Knowledge Base

- [Audio Effects Index](../../audio-effects/index.md)
- [Algorithms Index](../../algorithm/index.md)
- [Convolution Reverb](../../audio-effects/convolution-reverb.md)
- [Multiband EQ](../../audio-effects/multiband-eq.md)
