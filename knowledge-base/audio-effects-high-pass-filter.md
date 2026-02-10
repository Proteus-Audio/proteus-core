# Audio Effect: High‑Pass Filter

## What it is
A **high‑pass filter** that removes low frequencies below a cutoff point.

## How it behaves (plain language)
- Frequencies above the cutoff stay.
- Frequencies below the cutoff are reduced.
- The sound becomes **thinner / cleaner**.

## Frequency View (visual)

| Frequency Range | Output |
| --- | --- |
| Low | Attenuate |
| Mid | Pass (partial) |
| High | Pass |

```
Gain
1.0 ┤          ███████████
    │        ╱
    │      ╱
0.0 ┼────╱───────────────── Frequency
     cutoff
```

## Controls (conceptual)

| Control | What it changes | Audible effect |
| --- | --- | --- |
| `cutoff_hz` | Cutoff frequency | Higher = thinner |
| `enabled` | Bypass when false | Dry only |

## Typical use
- Remove rumble or mic handling noise
- Clean up low‑end buildup
