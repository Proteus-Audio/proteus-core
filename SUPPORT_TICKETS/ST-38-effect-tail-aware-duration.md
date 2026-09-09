# ST-38: Account for Effect Tails in Displayed Track Duration

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/dsp/effects/mod.rs` | Effects need a way to report whether they can extend audible output beyond source duration |
| `proteus-lib/src/dsp/effects/basic_reverb/` | Delay reverb has an explicit delay/feedback tail that can outlive input audio |
| `proteus-lib/src/dsp/effects/diffusion_reverb/` | Diffusion reverb can produce a decaying tail after input ends |
| `proteus-lib/src/dsp/effects/convolution_reverb/` | Convolution reverb tail depends on impulse-response length and tail threshold |
| `proteus-lib/src/playback/engine/mix/runner/effects_runtime.rs` | Runtime already drains effect tails after mix completion and should share duration semantics |
| `proteus-lib/src/playback/engine/mix/runner/loop_body.rs` | Tail drain limits and silence thresholds currently affect actual audible end time |
| `proteus-lib/src/container/prot/schedule.rs` | `.prot` duration scheduling currently uses source track durations without effect-tail extension |
| `proteus-lib/src/container/prot/helpers.rs` | Path/id duration aggregation should include selected effect-tail estimates where applicable |
| `proteus-lib/src/container/info/mod.rs` | Raw media duration should remain available, but callers need an adjusted audible duration path |
| `proteus-cli/src/cli/playback_runner.rs` | Playback progress and total duration display should use audible duration when effects extend output |
| `proteus-cli/src/cli/info_cmd.rs` and `proteus-cli/src/cli/ui.rs` | Info surfaces should distinguish raw media duration from effect-adjusted audible duration |

---

## Current state

Proteus now has accurate raw media duration probing for supported containers/codecs, but that duration
only describes the decoded source media. Several effects can continue producing audible output after
the source stream has ended:

- delay/basic reverb feeds silence through its delay line until the feedback tail decays
- diffusion reverb can continue ringing after the dry input stops
- convolution reverb can extend output by the impulse-response tail
- future effects may also introduce latency, lookahead, release, or tail behavior

The mix runtime already has a tail-drain path after `buffer_mixer.mix_finished()`, so playback can
continue after the source duration. However, displayed duration and schedule-derived duration still
come from the source/container duration. That makes progress bars, total time, seek limits, and
container duration display end too early when tail-producing effects are enabled.

### Why this matters

- Users can hear playback continue after the displayed duration reaches 100%
- Seeking near the end may clamp to the raw media duration instead of the audible end
- `.prot` containers with global or track effects can report a total duration shorter than playback
- Reverb/convolution tails should be predictable and testable instead of only discovered at runtime
- Duration display should remain stable and fast; it should not need to render the entire effect chain

## Recommended remediation

1. Define two duration concepts explicitly:
   - **media duration**: decoded source/container duration from `container::info`
   - **audible duration**: media duration plus any effect-produced tail or latency extension

2. Add effect-tail introspection to the DSP layer:
   - extend `DspEffect` or `AudioEffect` with a method such as `tail_duration_seconds(&EffectContext) -> Option<f64>`
   - return `Some(0.0)` for effects that cannot extend duration
   - return a conservative finite estimate for reverb-like effects
   - return `None` only when the tail cannot be estimated without rendering or loading missing assets
   - keep the method side-effect-free where possible

3. Implement tail estimates for current tail-producing effects:
   - delay/basic reverb: estimate from delay duration, feedback/amplitude decay, mix, and silence threshold
   - diffusion reverb: estimate from configured delay/diffusion parameters and decay threshold
   - convolution reverb: estimate from loaded impulse-response effective tail length, respecting `impulse_response_tail_db`
   - limiter/compressor lookahead or release should be audited, even if they do not extend output today

4. Add an effect-chain duration helper:
   - compute the total audible extension for a `Vec<AudioEffect>`
   - define how serial effects compose; conservative behavior is to sum extensions unless an effect can prove otherwise
   - ignore disabled or fully dry effects when they cannot produce audible tail
   - clamp invalid/NaN estimates to a safe fallback
   - expose enough detail for diagnostics: per-effect tail estimate and total extension

5. Apply adjusted duration to single-file playback:
   - raw `Info::new(file).duration_map` should continue to report media duration
   - playback/player duration used by progress display should include active effect-tail extension
   - `--read-durations` should remain raw media duration unless an explicit adjusted/effects-aware flag is added
   - seek and end-of-stream logic should use the right duration concept for each decision

6. Apply adjusted duration to `.prot` containers:
   - include global effects that apply to the whole container mix
   - include track/path effects when a selected source can produce a tail
   - account for shuffle schedules and multiple selected slots by extending the relevant segment/end time
   - avoid double-counting a global mix-tail once per track; global post-mix effects should extend the final mix once
   - preserve existing raw media duration maps for track lookup and scheduling inputs

7. Make CLI/TUI display explicit:
   - show raw media duration and audible/effects-adjusted duration where both are useful
   - progress bars should use audible duration during playback when effects are active
   - info views should label adjusted duration clearly, not silently replace raw duration
   - diagnostics should identify which effect(s) contributed tail extension

8. Align runtime tail drain with estimates:
   - audit `MAX_EFFECT_DRAIN_PASSES`, `DRAIN_SILENCE_EPSILON`, and `DRAIN_SILENT_PASSES_TO_STOP`
   - ensure estimated duration is not shorter than the runtime can audibly drain under normal settings
   - if runtime drain can stop early due to silence, estimates should use the same threshold or document why they differ
   - add a debug metric for actual drained tail duration so estimates can be compared during tests

9. Add regression coverage:
   - single file without effects reports equal media and audible durations
   - single file with delay reverb reports audible duration greater than media duration
   - convolution reverb with a known short impulse response extends audible duration by the expected tail
   - disabled/dry reverb does not extend audible duration
   - `.prot` with global reverb extends total container duration once
   - `.prot` with track-level reverb extends only affected selections
   - progress/end-of-stream tests do not mark playback complete before effect tail drain
   - raw duration APIs remain unchanged for callers that need media duration only

## Acceptance criteria

- [ ] Code distinguishes raw media duration from effects-adjusted audible duration
- [ ] Tail-producing effects expose a deterministic tail-duration estimate
- [ ] Effect-chain tail composition is documented and tested
- [ ] Single-file playback progress/total duration accounts for active effect tails
- [ ] `.prot` container duration accounts for global and track-level effect tails without double-counting
- [ ] CLI/TUI labels raw versus adjusted duration clearly
- [ ] Runtime tail drain behavior and duration estimates use compatible thresholds
- [ ] Tests cover reverb, convolution reverb, disabled/dry effects, single-file playback, and `.prot` containers

## Status

Not started.
