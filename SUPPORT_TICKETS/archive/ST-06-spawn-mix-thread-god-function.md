# SI-06: Mix Runner — `spawn_mix_thread` God Function

## Files affected

| File | Function | Lines |
|---|---|---|
| `proteus-lib/src/playback/engine/mix/runner/mod.rs` | `spawn_mix_thread` | ~536 |

The function significantly exceeds the 80-line hard limit. It performs the full lifecycle of
a mix thread — setup, decode worker management, DSP processing, and teardown — in a single
monolithic closure passed to `thread::spawn`.

> **Note**: SI-02 reduced `runner/mod.rs` from 715 to 586 lines by extracting the three
> helper functions (`drain_decode_events`, `apply_inline_track_mix_updates`,
> `apply_effect_runtime_updates`) to `loop_body.rs`. `runner/mod.rs` still exceeds the
> 600-line hard limit because `spawn_mix_thread` itself was not split. This ticket tracks
> that remaining work.

---

## `spawn_mix_thread` (~536 lines, lines 34–570)

### What it does

The function body (inside `thread::spawn`) executes six distinct phases:

1. **Runtime plan construction** — locks `prot`, builds the `RuntimeInstancePlan`, derives
   `EffectContext` and per-track mix settings. (~25 lines)

2. **Buffer sizing** — computes `start_samples`, `min_mix_samples`, and
   `convolution_batch_samples` from `buffer_settings` and the active effects chain. (~35 lines)

3. **`BufferMixer` construction** — builds `track_mix_by_logical`, creates the `BufferMixer`,
   and stores the `decode_backpressure` handle. (~25 lines)

4. **Decode worker spawning** — partitions sources into container track IDs vs standalone file
   paths, enables startup priority backpressure, spawns workers via
   `spawn_container_decode_worker` / `spawn_file_decode_worker`. (~65 lines)

5. **Effect warmup** — calls `effect.warm_up()` on every effect in the chain. (~12 lines)

6. **Main event loop** (~310 lines) — runs until abort or mix completion:
   - Drains the decode event channel (`loop_body::drain_decode_events`)
   - Applies inline track-mix and effect updates (`loop_body::apply_*`)
   - Blocks on the start gate until `start_samples` are buffered
   - Accumulates convolution batch samples when convolution is active
   - Calls `buffer_mixer.take_samples()` and pipes output through the DSP effect chain
   - Handles active inline effect transitions (crossfade between old/new chains)
   - Collects and reports DSP timing metrics (`#[cfg(feature = "debug")]`)
   - Drains the effect tail after the mix finishes
   - Sleeps 2 ms when no samples are ready

7. **Teardown** — writes finished track indices, shuts down backpressure, drops workers. (~13 lines)

### Why it is hard to split naively

The main loop (phase 6) closes over ~20 local variables initialised during phases 1–5:
`abort`, `packet_rx`, `buffer_mixer`, `decode_backpressure`, `effects`, `effect_context`,
`sender`, `buffer_notify`, `audio_info`, `dsp_metrics`, `convolution_batch_samples`,
`start_samples`, `min_mix_samples`, `started`, `last_effects_reset`,
`active_inline_transition`, `pending_mix_samples`, `effect_drain_*` counters, and several
`logged_first_*` flags. Passing these individually across a function boundary would create
an unworkable parameter list.

### Proposed extraction

Introduce a `MixLoopState` struct to bundle the mutable loop state, then split along the
natural phase boundaries:

```rust
/// All mutable state owned by the mix event loop.
struct MixLoopState {
    abort: Arc<AtomicBool>,
    packet_rx: mpsc::Receiver<DecodeWorkerEvent>,
    buffer_mixer: BufferMixer,
    decode_backpressure: Arc<DecodeBackpressure>,
    effects: Arc<Mutex<Vec<AudioEffect>>>,
    effect_context: EffectContext,
    sender: mpsc::SyncSender<(SamplesBuffer, f64)>,
    buffer_notify: Arc<Condvar>,
    audio_info: Info,
    dsp_metrics: Arc<Mutex<DspMetrics>>,
    inline_track_mix_updates: Arc<Mutex<Vec<InlineTrackMixUpdate>>>,
    inline_effects_update: Arc<Mutex<Option<InlineEffectsUpdate>>>,
    effects_reset: Arc<AtomicU64>,
    prot: Arc<Mutex<Prot>>,
    finished_tracks: Arc<Mutex<Vec<u16>>>,
    // sizing constants
    convolution_batch_samples: usize,
    start_samples: usize,
    min_mix_samples: usize,
    // mutable loop flags
    started: bool,
    last_effects_reset: u64,
    active_inline_transition: Option<ActiveInlineTransition>,
    pending_mix_samples: Vec<f32>,
    effect_drain_passes: usize,
    effect_drain_silent_passes: usize,
}
```

Then extract the following private functions in `runner/mod.rs` (or `loop_body.rs`):

```rust
/// Phases 1–5: build the runtime plan, size buffers, create the BufferMixer,
/// spawn decode workers, and warm up effects.
/// Returns None and sets abort if the instance plan is empty.
fn setup_mix_state(
    args: MixThreadArgs,
    sender: mpsc::SyncSender<(SamplesBuffer, f64)>,
    startup_trace: Instant,
) -> Option<MixLoopState>

/// Phase 6: run the main mix event loop until abort or mix completion.
fn run_mix_loop(state: &mut MixLoopState, startup_trace: Instant)

/// Phase 7: write finished track indices and shut down workers.
fn teardown_mix(state: MixLoopState, decode_workers: DecodeWorkerJoinGuard)
```

`spawn_mix_thread` then becomes a thin coordinator (~20 lines):

```rust
pub fn spawn_mix_thread(
    args: MixThreadArgs,
) -> (mpsc::Receiver<(SamplesBuffer, f64)>, JoinHandle<()>) {
    let (sender, receiver) = mpsc::sync_channel::<(SamplesBuffer, f64)>(1);
    let handle = thread::spawn(move || {
        let startup_trace = Instant::now();
        let Some(mut state) = setup_mix_state(args, sender, startup_trace) else {
            return;
        };
        run_mix_loop(&mut state, startup_trace);
        teardown_mix(state);
    });
    (receiver, handle)
}
```

Additionally, the DSP processing block inside the loop (the `if let Some(samples) = ...`
arm, currently ~140 lines) can be extracted as:

```rust
/// Apply the DSP effect chain to one batch of mixed samples and send to the sink.
/// Returns false if the sink has disconnected and the loop should abort.
fn process_and_send_samples(
    samples: Vec<f32>,
    state: &mut MixLoopState,
    startup_trace: Instant,
) -> bool
```

And the effect-tail drain arm (~65 lines):

```rust
/// Drain remaining effect tail samples after the mix finishes.
/// Returns false when the tail is exhausted or silent and the loop should exit.
fn drain_effect_tail(state: &mut MixLoopState) -> bool
```

---

## Notes

- `MixLoopState` should live in `loop_body.rs` alongside the extracted functions.
- All helper functions are private (`fn`, not `pub fn` or `pub(crate) fn`).
- `spawn_mix_thread` signature does not change — it remains the public entry point.
- The `#[cfg(feature = "debug")]` metric tracking inside the sample-processing block stays
  in `process_and_send_samples`; the conditional compilation attributes carry over unchanged.
- Existing tests should continue to pass without modification.

## Acceptance criteria

- [x] All existing tests pass (`cargo test -p proteus-lib`)
- [x] `cargo check --all-features` shows no new errors or warnings
- [x] `spawn_mix_thread` ≤ 30 lines
- [x] `setup_mix_state` ≤ 80 lines
- [x] `run_mix_loop` ≤ 80 lines
- [x] `process_and_send_samples` ≤ 80 lines
- [x] `drain_effect_tail` ≤ 80 lines
- [x] `runner/mod.rs` ≤ 400 lines

## Validation notes

Validated on March 11, 2026.

- `cargo test -p proteus-lib`: passed (`172` unit tests and `2` doc tests)
- `cargo check --all-features`: clean (one pre-existing dead_code warning in `types.rs`, zero new warnings)
- Function sizes after refactor:
  - `runner/mod.rs` total: 40 lines; `spawn_mix_thread`: 17 lines
  - `loop_body.rs` — `setup_mix_state`: 80, `run_mix_loop`: 29, `process_and_send_samples`: 60, `drain_effect_tail`: 39, `teardown_mix`: 17, `spawn_mix_decode_workers`: 37, `take_next_samples`: 27, `update_debug_metrics`: 20

### Split strategy

- Introduced `MixLoopState` struct in `loop_body.rs` to bundle all mutable loop state (~30 fields including `#[cfg(feature = "debug")]` metric accumulators and the `DecodeWorkerJoinGuard`).
- `setup_mix_state` (phases 1–5): builds the runtime plan, sizes buffers, constructs `BufferMixer`, spawns decode workers via extracted `spawn_mix_decode_workers`, and warms up effects. Returns `Option<MixLoopState>` — `None` on empty instance plan.
- `run_mix_loop` (phase 6): thin loop skeleton that calls `drain_decode_events`, `apply_inline_*`, start-gate check, and dispatches to `take_next_samples` → `process_and_send_samples` or `drain_effect_tail`.
- `take_next_samples`: convolution-batch accumulation and ring-buffer pop, extracted from the loop body to keep `run_mix_loop` concise.
- `process_and_send_samples`: effect-chain processing (including inline transition crossfade), `#[cfg(feature = "debug")]` metrics via extracted `update_debug_metrics`, and output send.
- `drain_effect_tail` (phase 6 tail): silence-detection loop for draining reverb tails after mix finishes.
- `teardown_mix` (phase 7): writes finished track indices, calls `decode_backpressure.shutdown()`, then explicitly drops `packet_rx` before `decode_workers` to preserve the deadlock-safe teardown order.
- The constants `MAX_EFFECT_DRAIN_PASSES`, `DRAIN_SILENCE_EPSILON`, and `DRAIN_SILENT_PASSES_TO_STOP` moved to `loop_body.rs` (as `pub(super)` so the test in `runner/mod.rs` can reference them).

## Status

SI-06 is complete. All acceptance criteria are met.
