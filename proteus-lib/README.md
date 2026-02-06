# Proteus Library

`proteus-lib` is the core Rust library behind the Proteus audio tools. It provides:
- Container parsing for `.prot`/`.mka` files.
- A real-time playback engine and high-level `Player` API.
- DSP utilities (convolution reverb and impulse response loading).
- Diagnostics and benchmark helpers (optional feature flags).

**Quick Start**
```rust
use proteus_lib::playback::player::Player;

fn main() {
    let file_path = "/path/to/file.prot".to_string();
    let mut player = Player::new(&file_path);
    player.play();
    player.sleep_until_end();
}
```

## Structure

**`container/`**
- Parses `.prot`/`.mka` containers, reads `play_settings.json`, and exposes track selections.
- `info.rs` handles probe metadata, duration lookup, and full packet scanning.
- `play_settings/` models multiple schema versions via serde.

**`playback/`**
- `player.rs` is the main public API (`Player`) that manages state and threads.
- `engine/` contains the lower-level mixing engine, buffer management, and reverb worker.

**`dsp/`**
- `convolution.rs` provides FFT-based convolution (complex or real FFT).
- `reverb.rs` wraps convolution into a reusable per-channel reverb.
- `impulse_response.rs` loads and normalizes impulse responses from files or container attachments.

**`audio/`**
- Shared ring buffers and sample helpers used by the engine.

**`diagnostics/`**
- Optional benchmark utilities and a `Reporter` to emit playback status snapshots.

## Playback Model (High Level)

1. A `Prot` container resolves track selections and metadata.
2. Track decoder threads feed per-track ring buffers.
3. A mixing thread combines buffers into interleaved audio.
4. Optional convolution reverb is applied (if enabled).
5. The `Player` sends mixed audio to the output sink.

## Impulse Responses

Impulse responses can be loaded from:
- a file path (`file:ir.wav` or plain path)
- a `.prot`/`.mka` attachment (`attachment:ir.wav`)

Tail trimming defaults to `-60 dB` and can be overridden via settings or at runtime.

## Feature Flags

- `bench`: enables synthetic DSP benchmarks.
- `real-fft`: uses real FFTs for convolution instead of complex FFTs.

## Notes

- The library is designed for real-time playback and integrates with the CLI and GUI apps.
- Many settings are driven by the `play_settings.json` embedded in container files.
