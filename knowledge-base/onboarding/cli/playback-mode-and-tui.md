# CLI Onboarding: Playback Mode and TUI Loop

- [Back to CLI Onboarding](./index.md)

## Default Playback Mode (No Subcommand)

In [`proteus-cli/src/runner.rs`](../../../proteus-cli/src/runner.rs), playback mode is the path where:

- no explicit subcommand is chosen
- an `INPUT` path is provided

The runner then:

1. Resolves input type (container, directory, or single file)
2. Builds a `proteus-lib` `Player`
3. Applies runtime tuning flags (buffer sizes, fades, logging knobs)
4. Optionally loads an effects JSON file
5. Starts playback and enters the UI/input loop

## How `Player` Is Chosen

The runner branches to:

- `Player::new_with_options(...)` for containers
- `Player::new_from_file_paths_with_options(...)` for directory mode
- `Player::new_from_file_paths_legacy_with_options(...)` for plain files

This is the main place to change CLI-side playback initialization behavior.

## TUI Lifecycle

Still in `runner.rs`:

- raw terminal mode is enabled
- alternate screen may be entered unless `--quiet`
- logs can be captured and rendered into the TUI
- a loop repeatedly reads player state and renders status

The UI loop also polls keyboard input through [`proteus-cli/src/controls.rs`](../../../proteus-cli/src/controls.rs).

## Keyboard Controls (Current Defaults)

Handled in `handle_key_event(...)`:

- `space`: play/pause
- `q` / `Ctrl-C`: stop + exit
- `s`: shuffle
- left/right arrows: seek ±5s
- `r`: toggle reverb enabled
- `-` / `=`: adjust reverb mix

These are CLI conveniences layered on top of `Player` APIs.

## Status Text vs Rendering

There is a clean split:

- `controls::status_text(...)` builds a status string from runtime data
- `ui::draw_status(...)` renders it using Ratatui

This makes status-content changes easier without editing layout code.

## Debug/Feature-Flag Surfaces

`runner.rs`, `controls.rs`, and `ui.rs` use feature-gated fields/rendering:

- `debug` feature adds extra runtime metrics and internal state display
- `output-meter` feature enables level meters in the TUI

Be careful when changing shared status structures because feature-gated fields can break non-debug builds if not kept consistent.

## Related

- [CLI Architecture and Control Flow](./architecture-and-control-flow.md)
- [Library Playback Runtime and Threading](../library/playback-runtime-and-threading.md)
- [Player Data Flows](../../player/data-flows.md)
