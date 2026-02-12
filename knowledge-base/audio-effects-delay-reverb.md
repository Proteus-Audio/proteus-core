# Audio Effect: Delay Reverb

## What it is
A classic **delay‑based reverb** that feeds the signal through short delays with feedback to create a spacious echo tail. This is the effect formerly labeled “Basic Reverb” in the codebase and is still available via the deprecated `BasicReverb` alias.

## How it behaves (plain language)
- The input is copied into one or more delay lines.
- A portion of the delayed signal is fed back to create multiple echoes.
- The dry (original) and wet (delayed) signals are mixed together.

## How it works (step‑by‑step)
1. Compute the delay length in samples from `duration_ms`, sample rate, and channel count.
2. Allocate a circular delay line buffer of that length and track a write cursor.
3. Choose the feedback amplitude: if `mix` is set, use `mix` (clamped); otherwise use the stored `amplitude`.
4. For each incoming sample:
5. Read the delayed sample at the cursor.
6. Output `sample + (delayed * amplitude)`.
7. Write `sample + (delayed * amplitude)` back into the delay line (feedback).
8. Advance the cursor, wrapping at the buffer end.
9. If draining, keep feeding zeros through the delay line to emit the remaining tail.

## Signal Flow (simplified)

```
Input ──┬─────────────► Dry ─────────────┐
        │                                ├─► Mix ─► Output
        └─► Delay(s) + Feedback ─► Wet ───┘
```

## Controls (conceptual)

| Control | What it changes | Audible effect |
| --- | --- | --- |
| `dry_wet` | Blend between dry and wet | More/less reverb presence |
| `enabled` | Bypass when false | Dry only |

## Technical
This effect is a feedback delay-line reverb, a simple form of a **Schroeder-style artificial reverb**. The core idea is to use short delays plus feedback to create exponentially decaying echoes that psychoacoustically read as room sustain instead of discrete repeats.

The algorithmic precedent is classic digital reverberation work from the 1960s and 1970s: cascading/parallel delay structures with feedback are the historical foundation of many lightweight reverbs. This implementation uses the minimal version of that family (single delay feedback path with wet/dry mix), trading realism for low CPU and predictable behavior.

## Typical use
- Add subtle space to dry signals
- Thicken short percussive material

## Key properties

| Property | Value |
| --- | --- |
| Latency | Low (short delays) |
| CPU cost | Low |
| Tail length | Adjustable via feedback |

## Notes
- The public name is **DelayReverb**. The old **BasicReverb** name remains as a deprecated alias for compatibility.
