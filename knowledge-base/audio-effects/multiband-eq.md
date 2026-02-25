# Audio Effect: Multiband EQ

## What it is
A **parametric multiband EQ** with:
- `points: Vec<...>` for any number of peaking EQ bands
- optional `low_edge` filter: `high_pass` or `low_shelf`
- optional `high_edge` filter: `low_pass` or `high_shelf`

## How it behaves (plain language)
- Each point boosts/cuts around its own center frequency.
- `low_edge` shapes low-end boundaries.
- `high_edge` shapes high-end boundaries.

## How it works (step-by-step)
1. Sanitize frequencies, Q, and gain values to safe ranges.
2. Convert each point to a peaking biquad.
3. Convert optional edge settings to low/high-pass or shelf biquads.
4. For each sample, process in order: `low_edge` -> all `points` -> `high_edge`.

## Controls (conceptual)

| Control | What it changes | Audible effect |
| --- | --- | --- |
| `points[].freq_hz` | Center frequency for a point | Moves where boost/cut happens |
| `points[].q` | Bandwidth of a point | Higher = narrower |
| `points[].gain_db` | Boost/cut of a point | Positive boosts, negative cuts |
| `low_edge` | Optional low-end boundary | HP removes rumble, low shelf tilts lows |
| `high_edge` | Optional high-end boundary | LP removes highs, high shelf tilts highs |
| `enabled` | Bypass when false | Dry only |

## Technical
The effect is a cascade of **second-order IIR biquads** (peaking + optional shelves/passes), keeping per-sample cost low and suitable for real-time playback.

## Typical use
- Broad tonal correction with many narrow points.
- Add surgical cuts while also applying end-band tone shaping.
