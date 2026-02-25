# CLI Onboarding: Common CLI Change Tasks

- [Back to CLI Onboarding](./index.md)

## 1) Add a New CLI Flag for Playback Tuning

Touch at least:

- [`proteus-cli/src/cli/args.rs`](../../../proteus-cli/src/cli/args.rs)
- [`proteus-cli/src/runner.rs`](../../../proteus-cli/src/runner.rs)

Pattern:

1. Add Clap arg + default/help text
2. Parse in `runner::run(...)`
3. Apply to `Player` before `player.play()`
4. Verify behavior in normal and `--quiet` modes

## 2) Add a New Keyboard Shortcut

Touch:

- [`proteus-cli/src/controls.rs`](../../../proteus-cli/src/controls.rs)
- [`proteus-cli/src/ui.rs`](../../../proteus-cli/src/ui.rs) (update help text)

Keep the control mapping and the rendered help line synchronized.

## 3) Add a New Subcommand

Touch:

- [`proteus-cli/src/cli/args.rs`](../../../proteus-cli/src/cli/args.rs)
- [`proteus-cli/src/runner.rs`](../../../proteus-cli/src/runner.rs)
- optionally a new module under [`proteus-cli/src/cli/`](../../../proteus-cli/src/cli/)

Prefer a dedicated module when logic is substantial (same pattern as `bench` / `verify`).

## 4) Change Directory Project Format

Touch:

- [`proteus-cli/src/project_files.rs`](../../../proteus-cli/src/project_files.rs)
- [`proteus-lib/src/container/prot.rs`](../../../proteus-lib/src/container/prot.rs) (if `PathsTrack` semantics change)

Be explicit about backward compatibility for existing `shuffle_schedule.json` files.

## 5) Improve TUI Status / Metrics

Touch:

- [`proteus-cli/src/runner.rs`](../../../proteus-cli/src/runner.rs) (collect runtime data)
- [`proteus-cli/src/controls.rs`](../../../proteus-cli/src/controls.rs) (status string structure)
- [`proteus-cli/src/ui.rs`](../../../proteus-cli/src/ui.rs) (layout/rendering)

Feature flags (`debug`, `output-meter`) can affect what fields exist and what gets rendered.

## Related

- [Playback Mode and TUI Loop](./playback-mode-and-tui.md)
- [Library Playback Runtime and Threading](../library/playback-runtime-and-threading.md)
- [Player: `run_playback_thread` Sample Processing Flow](../../player/run-playback-thread-sample-flow.md)
