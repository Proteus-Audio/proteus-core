# SI-11 — `too_many_arguments` suppressions

## Rule

Style Guide §5 (Function and Type Design): "Prefer ≤ 4 parameters. Bundle related
parameters into a struct when you need more."

## Problem

Fifteen functions across the codebase suppress the `clippy::too_many_arguments`
lint rather than grouping their parameters into a purpose-built struct. The most
egregious example — `MixLoopState::new` — has **19 parameters**. Suppressing the
lint papers over the smell without fixing it.

Current occurrences (`#[allow(clippy::too_many_arguments)]`):

| File | Function |
| --- | --- |
| `container/prot/plan.rs` (×2) | `collect_slot_instances`, one other helper |
| `container/prot/helpers.rs` | private helper |
| `container/prot/schedule.rs` | private helper |
| `playback/engine/mix/runner/startup.rs` (×3) | `setup_mix_state`, `spawn_mix_decode_workers`, one other helper |
| `playback/engine/mix/runner/decode/container_worker.rs` (×2) | decode worker constructors |
| `playback/engine/mix/runner/decode/file_worker.rs` (×2) | decode worker constructors |
| `playback/engine/mix/runner/state.rs` | `MixLoopState::new` (19 parameters) |
| `playback/engine/mix/runner/decode/mod.rs` | `forward_decoded_packet` (10 parameters) |
| `playback/engine/mix/buffer_mixer/backpressure.rs` | backpressure helper |
| `playback/engine/mix/buffer_mixer/packet_router.rs` (×3) | routing helpers |

## Fix

For each function, introduce a `*Config` or `*Args` struct that bundles the
related parameters, then update all call sites. `MixLoopState::new` is the
largest offender and the most valuable target; start there since reducing its
argument list also simplifies the startup code path that calls it.

Where multiple helpers in the same module share the same long parameter list, a
single shared args struct can serve all of them.

Remove the `#[allow(clippy::too_many_arguments)]` suppressions once the
parameters are bundled.
