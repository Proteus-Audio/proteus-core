# Effects Metering CLI Guide

This guide covers the offline effect-metering command added to `proteus-cli`.

The command runs audio through an effects chain without opening the normal
playback UI, then reports per-effect **before/after** metering.

## Features

The metering command is behind opt-in CLI features.

Build with time-domain effect metering:

```bash
cargo build -p proteus-cli --features effect-meter-cli
```

Build with time-domain metering plus spectral reporting:

```bash
cargo build -p proteus-cli --features effect-meter-cli-spectral
```

If you run the command without the required feature, the CLI exits with:

```text
Effect metering requires the `effect-meter-cli` feature.
```

## Command Shape

```bash
prot meter effects <INPUT> \
  [--effects-json <PATH>] \
  [--seek <SECONDS>] \
  [--duration <SECONDS>] \
  [--format table|bars|json] \
  [--summary final|max] \
  [--spectral]
```

Arguments:

- `INPUT`: audio file or container file to inspect
- `--effects-json`: JSON file containing `Vec<AudioEffect>`
- `--seek`: optional start offset in seconds
- `--duration`: optional capture duration in seconds after `--seek`
- `--format`: `table`, `bars`, or `json`
- `--summary`: `final` or `max`
- `--spectral`: append spectral bucket output when built with `effect-meter-cli-spectral`

Notes:

- The command is offline and does not require a live audio device.
- If `--effects-json` is omitted, the command runs with an empty effects chain.
- Directory inputs are not supported by this offline metering path.

## Creating An Effects File

Generate a starter effects JSON file:

```bash
cargo run -p proteus-cli -- create effects-json > effects.json
```

That emits a full default chain. For focused metering tests, a smaller file is
usually easier to reason about.

Example `gain.json`:

```json
[
  {
    "GainSettings": {
      "enabled": true,
      "gain": 2.0
    }
  }
]
```

Example `highpass.json`:

```json
[
  {
    "HighPassFilterSettings": {
      "enabled": true,
      "freq_hz": 1000,
      "q": 0.707
    }
  }
]
```

## Table Output

Show one row per effect with input/output peak and RMS deltas:

```bash
cargo run -p proteus-cli --features effect-meter-cli -- \
  meter effects test_audio/test-16bit.wav \
  --effects-json gain.json \
  --duration 0.25 \
  --format table
```

Typical output:

```text
input=test_audio/test-16bit.wav sample_rate=44100Hz channels=2 frames=11025 summary=Max
idx effect               in_peak out_peak   delta   in_rms  out_rms   delta
0   Gain                  -14.5    -8.5    +6.0    -22.1    -16.1    +6.0
```

Use this when you want a quick scan of gain staging through the chain.

## Bars Output

Render a more visual before/after view:

```bash
cargo run -p proteus-cli --features effect-meter-cli -- \
  meter effects test_audio/test-16bit.wav \
  --effects-json gain.json \
  --duration 0.25 \
  --format bars
```

Typical output:

```text
input=test_audio/test-16bit.wav sample_rate=44100Hz channels=2 frames=11025 summary=Max
[0] Gain
in : L [##          ] -14.5 dBFS | R [##          ] -13.8 dBFS
out: L [####        ]  -8.5 dBFS | R [####        ]  -7.8 dBFS
Δpk= +6.0  Δrms= +6.0
```

Use this when you want to see the boundary change directly.

## JSON Output

Emit a deterministic machine-readable report for tests or tooling:

```bash
cargo run -p proteus-cli --features effect-meter-cli -- \
  meter effects test_audio/test-16bit.wav \
  --effects-json gain.json \
  --duration 0.25 \
  --format json \
  --summary max
```

Example shape:

```json
{
  "input_path": "test_audio/test-16bit.wav",
  "sample_rate": 44100,
  "channels": 2,
  "frames_processed": 11025,
  "summary_mode": "Max",
  "effects": [
    {
      "effect_index": 0,
      "effect_name": "Gain",
      "levels": {
        "input": {
          "peak": [0.15914917, 0.18911743],
          "rms": [0.07845963, 0.082467616]
        },
        "output": {
          "peak": [0.31829834, 0.37823486],
          "rms": [0.15691926, 0.16493523]
        }
      }
    }
  ],
  "spectral": null
}
```

This is the best format for regression tests and scriptable inspection.

## Summary Modes

`--summary final`:

- keeps the last captured per-effect snapshot
- useful when you care about the end state of the inspected window

`--summary max`:

- keeps the maximum observed per-channel values across the inspected window
- useful for quick assertions like “output peak should be higher than input peak”

Examples:

```bash
cargo run -p proteus-cli --features effect-meter-cli -- \
  meter effects test_audio/test-16bit.wav \
  --effects-json gain.json \
  --duration 1.0 \
  --summary final
```

```bash
cargo run -p proteus-cli --features effect-meter-cli -- \
  meter effects test_audio/test-16bit.wav \
  --effects-json gain.json \
  --duration 1.0 \
  --summary max
```

## Seek And Windowing

Inspect a specific region of the file:

```bash
cargo run -p proteus-cli --features effect-meter-cli -- \
  meter effects test_audio/test-16bit.wav \
  --effects-json gain.json \
  --seek 12.5 \
  --duration 0.5 \
  --format table
```

This skips the first `12.5` seconds and meters the next `0.5` seconds.

## Spectral Reporting

Spectral output is only available when built with `effect-meter-cli-spectral`.

Example:

```bash
cargo run -p proteus-cli --features effect-meter-cli-spectral -- \
  meter effects test_audio/test-16bit.wav \
  --effects-json highpass.json \
  --duration 0.25 \
  --format table \
  --spectral
```

The normal time-domain table is still printed first. A spectral section is then
appended for supported filter effects.

JSON example:

```bash
cargo run -p proteus-cli --features effect-meter-cli-spectral -- \
  meter effects test_audio/test-16bit.wav \
  --effects-json highpass.json \
  --duration 0.25 \
  --format json \
  --spectral
```

Supported spectral effects:

- `LowPassFilterSettings`
- `HighPassFilterSettings`
- `MultibandEqSettings`

## Common Workflows

Check whether a gain effect actually boosts the signal:

```bash
cargo run -p proteus-cli --features effect-meter-cli -- \
  meter effects test_audio/test-16bit.wav \
  --effects-json gain.json \
  --duration 0.25 \
  --format table \
  --summary max
```

Inspect multiple effects in order:

```bash
cargo run -p proteus-cli --features effect-meter-cli -- \
  meter effects test_audio/test-16bit.wav \
  --effects-json effects_chain.json \
  --duration 0.5 \
  --format bars
```

Capture JSON for downstream processing:

```bash
cargo run -p proteus-cli --features effect-meter-cli -- \
  meter effects test_audio/test-16bit.wav \
  --effects-json effects_chain.json \
  --duration 0.5 \
  --format json > meter-report.json
```

## Troubleshooting

`Effect metering requires the effect-meter-cli feature`

- rebuild with `--features effect-meter-cli`

`Spectral effect metering requires the effect-meter-cli-spectral feature`

- rebuild with `--features effect-meter-cli-spectral`

`Failed to load effects json`

- check that the file exists
- check that the JSON is a `Vec<AudioEffect>`
- make sure the variant names match the serialized effect names such as
  `GainSettings`, `HighPassFilterSettings`, or `MultibandEqSettings`

`No effects configured.`

- you ran the command without `--effects-json`
- add an effects file if you want per-effect rows
