# Engine Mix Refactor Plan

## Goal

Refactor the mix/streaming pipeline to use one consistent decode->route->mix model with a cleaner/clearer signal chain so debugging and troubleshooting are straightforward, and so that:

- EOF detection is deterministic and cannot be missed by path-specific behavior.
- all logical tracks remain sample-aligned, including shuffle transitions and duplicate selections.
- container and file-path playback share the same orchestration model.
- shuffle behavior remains correct when `selections_count > 1`.

## Problem Summary

Current behavior has multiple decode/mix paths (container fast-path vs per-source workers), which increases divergence risk:

- EOF state is inferred through multiple signals and path-specific conditions.
- track alignment can drift when source workers advance independently in edge cases.
- shuffle transitions create additional runtime-key complexity.

## Target Architecture (single in/out emphasis)

### 1) Build schedule first, then derive concrete buffer instances

After runtime shuffle schedule is built, derive a deterministic list of **buffer instances** from schedule entries.

Important: duplicates are instances, not deduped by source id/path.

Examples (from requirement):

- `[(0.0, [["1"], ["3"]]), (14.604, [["2"], ["3"]])]` -> instances for `"1"`, `"2"`, `"3"`
- `[(0.0, [["1","2"], ["3"]]), (14.604, [["2","4"], ["3"]])]` -> instances for `"1"`, `"2"`, `"4"`, `"3"`
- `[(0.0, [["1","2"], ["3"]]), (14.604, [["2","4"], ["2"]])]` -> 5 instances (`"1"`, `"2"`, and `"4"` for Logical Track 1), (`"3"` and `"2"`for Track 2)
- `[(0.0, [["1","2"], ["3"]]), (14.604, [["2","2"], ["2"]])]` -> 5 instances (`"1"`, `"2"`, and `"2"` for Logical Track 1… Track two is duplicated because it exists twice in the same track at the same time), (`"3"` and `"2"`for Track 2)

To support these cases safely, represent each occurrence with an explicit `InstanceId`, even if source id/path matches another instance.

### 2) Introduce intermediary orchestrator for buffer routing

Create a dedicated intermediary struct that owns routing decisions and buffer-fill behavior.

Proposed module:

- `proteus-lib/src/playback/engine/mix/schedule_router.rs`

Proposed primary struct:

- `ScheduleBufferRouter`

Responsibilities:

- own buffer-instance metadata (instance id, source key, logical track index, active windows).
- own the per-instance ring buffers (or references to them).
- route decoded sample packets to the right instance buffers.
- fill inactive-but-live instances with zeros to keep all instance timelines aligned.
- determine whether a decoded source can be ignored at a given timestamp.
- provide deterministic per-instance EOF state.

Core API to implement (requirement-specific):

- `route_packet(samples: &[f32], source: SourceKey, packet_ts: f64) -> RouteDecision`

Where `RouteDecision` includes:

- target instance ids to receive decoded samples,
- instance ids to zero-fill for that packet time span,
- source ignored flag when source is irrelevant for active/future windows.

### 3) Decoder topology

Use a unified decoder strategy:

- **Single container file mode** (`.prot/.mka`): one container decode worker reads packets and calls router.
- **File-path mode**: one decode worker per unique file path reads packets and calls router.

In both cases, decode workers become producers of `(samples, source_key, packet_ts)` events; router decides destination instance buffers.

### 4) Two-stage mixing model

After instance buffers are aligned and routed:

1. **Logical track premix stage**
- mix all instance buffers belonging to one logical track into one logical-track buffer.

2. **Per-track DSP stage**
- apply per-logical-track gain + pan in a small chain.
- add new `Pan` DSP effect (stereo-only now, with comment placeholder for future multi-channel extension).

3. **Final mix stage**
- combine logical-track outputs in equal weight (or configurable policy), then run global effects chain, then sink.

This preserves modularity and makes per-track vs global DSP boundaries explicit.

## Data Model Changes

## New types

- `SourceKey`
: `TrackId(u32)` or `FilePath(String)`.

- `InstanceId`
: unique per occurrence in schedule expansion.

- `BufferInstanceMeta`
: `{ instance_id, logical_track_index, source_key, active_windows, selection_index, occurrence_index }`.

- `ActiveWindow`
: `{ start_ms, end_ms }` (`end_ms = None` for tail).

- `RouteDecision`
: `{ sample_targets, zero_fill_targets, ignored }`.

## Existing schedule API

Keep current grouped schedule API (`Vec<(f64, Vec<Vec<String>>)`) for UI, and add a runtime expansion API that maps grouped schedule -> instance metadata/windows.

Proposed additions in `container/prot.rs`:

- `build_runtime_instance_plan(start_time: f64) -> RuntimeInstancePlan`

`RuntimeInstancePlan` should include:

- flattened slot ordering used by mixer,
- instance metadata list,
- event boundaries.

## Step-by-step Implementation Plan

## Phase 0: Guardrails and observability

1. Add debug counters for:
- per-instance produced samples,
- per-instance zero-filled samples,
- per-instance EOF reached timestamp,
- router drops/ignores.
2. Add temporary structured logging around shuffle boundary transitions.

Exit criteria:

- current behavior still passes `cargo check` and existing tests.

## Phase 1: Introduce router and runtime instance plan (no behavior switch yet)

1. Add `schedule_router.rs` with `ScheduleBufferRouter` and `RouteDecision`.
2. Add schedule expansion from grouped logical tracks to concrete instances + active windows.
3. Add unit tests for expansion and routing decisions, including duplicate-source scenarios from requirements.

Exit criteria:

- router tests pass,
- no runtime path switched yet.

## Phase 2: Unify decode producers to packet events

1. Add a small internal packet-event interface used by both decoder types:
- `DecodedPacket { source_key, packet_ts, samples, eos_flag }`.
2. Adapt container decoder path to emit these events.
3. Adapt file-path decoders to emit these events.
4. Keep old direct buffer writes behind compatibility flag while introducing new path.

Exit criteria:

- both decoder sources can feed router in test harness.

## Phase 3: Router-owned buffer filling + zero-fill alignment

1. Move instance-buffer write responsibility into router.
2. Implement zero-fill logic for inactive windows per instance.
3. Implement ignore logic for irrelevant packets/sources.
4. Implement deterministic per-instance EOF completion and aggregate EOF signal.

Exit criteria:

- deterministic EOF in integration tests,
- no per-path EOF branch differences.

## Phase 4: Replace runtime mix input model

1. Replace active/fading runtime-key model with instance-driven model.
2. Remove shuffle-time key rotation in hot loop; shuffle is represented by precomputed active windows.
3. Mixer consumes aligned instance buffers using one invariant: all live instances progress on same timeline.

Exit criteria:

- no `next_track_key`/fading-key runtime mutation needed for scheduling correctness,
- boundary sync tests pass.

## Phase 5: Two-stage mix + per-track pan/gain chain

1. Implement logical-track premix stage.
2. Add new `Pan` effect under `proteus-lib/src/dsp/effects/pan.rs`.
3. Add `Pan` to `AudioEffect` enum and serde wiring.
4. Apply per-track chain: `Gain` + `Pan` (stereo-only now).
5. Mix logical tracks to final buffer and run global effects chain as today.

Exit criteria:

- gain/pan tests pass,
- output parity acceptable where pan/gain unchanged.

## Phase 6: Remove legacy branches and clean up

1. Remove container fast-path branching that bypasses router model.
2. Remove obsolete shuffle runtime key/fading structures if no longer needed.
3. Update knowledge-base docs and inline comments to reflect new model.

Exit criteria:

- one decode-routing-mix model in code,
- lower cyclomatic complexity in `mix/runner.rs`.

## Testing Plan

## Unit tests

- schedule expansion into instances:
- duplicates at same timestamp,
- duplicates across logical tracks,
- `selections_count > 1` mappings.
- router decisions:
- packet routes to correct instances,
- inactive instances get zero-fill,
- irrelevant source is ignored.
- pan effect:
- stereo pan law behavior,
- bypass behavior,
- TODO comment for multi-channel extension.

## Integration tests

- container mode: one decoder drives multiple instances; no desync over shuffle boundaries.
- file-path mode: one decoder per path; alignment maintained.
- EOF:
- all instances EOF -> engine drains exactly once,
- no hung playback with pending schedule windows.
- seek/start-time:
- start mid-schedule preserves alignment and EOF behavior.

## Regression checks

- `cargo check`
- `cargo test -p proteus-lib --lib`
- manual playback with debug metrics (`--features debug`) for underrun/eof counters.

## Proposed Module Layout

- `proteus-lib/src/playback/engine/mix/schedule_router.rs` (new)
- `proteus-lib/src/playback/engine/mix/decoder_events.rs` (new, optional)
- `proteus-lib/src/playback/engine/mix/track_stage.rs` (new, logical-track premix)
- `proteus-lib/src/dsp/effects/pan.rs` (new)
- `proteus-lib/src/playback/engine/mix/runner.rs` (simplified orchestration)

## Risks and Mitigations

- Risk: zero-fill policy increases CPU/memory.
- Mitigation: fill by chunk, avoid per-sample branching, reuse buffers.

- Risk: semantics change for crossfade feel.
- Mitigation: keep existing crossfade as optional layer after alignment is stable.

- Risk: schedule-window math bugs.
- Mitigation: property-style unit tests for window expansion and boundary math.

## Deliverables

- [ ] Router + runtime instance plan implementation.
- [ ] Unified decoder event ingestion.
- [ ] Two-stage logical-track mix pipeline.
- [ ] New stereo `Pan` effect integrated into per-track stage.
- [ ] Removed legacy divergent paths and updated docs.
