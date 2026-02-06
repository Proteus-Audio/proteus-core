# Proteus CLI

Command-line player for `.prot` and `.mka` containers powered by `proteus-lib`.

**Usage**
- `cargo run -p proteus-cli -- /path/to/file.prot`
- `cargo run -p proteus-cli -- /path/to/file.mka`
- `cargo run -p proteus-cli -- /path/to/file.prot --ir /path/to/ir.wav`

**Controls**
- `space` play/pause
- `s` shuffle
- `←/→` seek 5s
- `r` toggle reverb
- `-` / `=` adjust reverb mix
- `q` quit

**Debug**
- `cargo run -p proteus-cli --features debug -- /path/to/file.prot`
- `RUST_LOG=debug` enables debug logging

**Notes**
- Logs are captured and shown in the TUI while the interface is active.
