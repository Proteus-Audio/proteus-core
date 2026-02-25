# Library Onboarding: Common Change Tasks

- [Back to Library Onboarding](./index.md)

## 1) Add a New Audio Effect

Primary files:

- [`proteus-lib/src/dsp/effects/mod.rs`](../../../proteus-lib/src/dsp/effects/mod.rs)
- [`proteus-lib/src/playback/player/effects.rs`](../../../proteus-lib/src/playback/player/effects.rs)
- [`proteus-cli/src/project_files.rs`](../../../proteus-cli/src/project_files.rs) (CLI defaults/init files)

Recommended sequence:

1. Implement effect module + settings parser/serde shape
2. Add `AudioEffect` enum variant and dispatch wiring
3. Add player-facing helpers (optional but often useful)
4. Add CLI default-effect-chain support if appropriate
5. Add/update knowledge-base pages and links

## 2) Change Playback Buffering / Startup Behavior

Primary files:

- [`proteus-lib/src/playback/player/settings.rs`](../../../proteus-lib/src/playback/player/settings.rs)
- [`proteus-lib/src/playback/player/runtime/worker/runner.rs`](../../../proteus-lib/src/playback/player/runtime/worker/runner.rs)
- [`proteus-lib/src/playback/engine/mod.rs`](../../../proteus-lib/src/playback/engine/mod.rs)

Watch for regressions in:

- startup latency
- underruns / sink starvation
- seek behavior
- effect tail draining

## 3) Change Shuffle Semantics

Primary files:

- [`proteus-lib/src/container/prot.rs`](../../../proteus-lib/src/container/prot.rs)
- [`proteus-lib/src/playback/engine/mix/runner/mod.rs`](../../../proteus-lib/src/playback/engine/mix/runner/mod.rs)

Guidance:

- Keep container-layer schedule generation and runtime application semantics aligned.
- Re-read [Shuffle Points in Playback](../../player/shuffle-points-playback.md) before changing behavior.

## 4) Investigate Clicks/Glitches

Start here:

- [`proteus-lib/src/playback/engine/mix/output_stage.rs`](../../../proteus-lib/src/playback/engine/mix/output_stage.rs)
- [`proteus-lib/src/playback/player/runtime/worker/runner.rs`](../../../proteus-lib/src/playback/player/runtime/worker/runner.rs)
- effect-specific modules (especially reverbs/convolution)

Then check knowledge-base notes:

- [Convolution Reverb Boundary Discontinuity](../../convolution-reverb/boundary-discontinuity.md)
- [Run Playback Thread Sample Flow](../../player/run-playback-thread-sample-flow.md)

## 5) Add Developer Documentation

When behavior changes:

- update the relevant effect/algorithm/player KB page
- update onboarding docs if the change affects architecture or developer workflows

## Related

- [Architecture Overview](./architecture-overview.md)
- [Playback Runtime and Threading](./playback-runtime-and-threading.md)
- [DSP Effects and Signal Chain](./dsp-effects-and-signal-chain.md)
