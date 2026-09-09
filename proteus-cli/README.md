# Proteus CLI

Command-line player for `.prot` and `.mka` containers powered by `proteus-lib`.

**Usage**
- `cargo run -p proteus-cli -- /path/to/file.prot`
- `cargo run -p proteus-cli -- /path/to/file.mka`

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

**Benchmarking**

Benchmark offline DSP processing for an audio or `.prot` file with:

```text
cargo run --release -p proteus-cli -- bench /path/to/file.prot
```

The Markdown report is printed to standard output unless `--output` (or `-o`) is
provided, for example `prot bench song.wav -o benchmark.md`. Each report compares
no effects, each available effect, and—when the input is a `.prot`—its embedded
effects chain. Decode and `.prot` settings-load time are reported separately;
the timing table measures only DSP work on already-decoded PCM. For multi-track
`.prot` containers, the first decodable audio stream supplies the representative
PCM while the embedded project chain is measured in full. Use a release build
for comparable performance measurements.

**CLI Options**
```text
Usage: proteus-cli [OPTIONS] [INPUT]

Arguments:
  [INPUT]  The input file path, or - to use standard input

Options:
  -s, --seek <TIME>                    Seek to the given time in seconds
  -g, --gain <GAIN>                    The playback gain [default: 70]
      --bench-dsp                      Run a synthetic DSP benchmark and exit
      --bench-sweep                    Run a sweep over multiple FFT sizes and exit
      --bench-fft-size <SIZE>          FFT size for DSP benchmark [default: 24576]
      --bench-input-seconds <SECONDS>  Input length in seconds for DSP benchmark [default: 1.0]
      --bench-ir-seconds <SECONDS>     Impulse response length in seconds for DSP benchmark [default: 2.0]
      --bench-iterations <COUNT>       Number of iterations for DSP benchmark [default: 5]
      --start-buffer-ms <MS>           Amount of audio (ms) to buffer before starting playback [default: 20]
      --track-eos-ms <MS>              Heuristic end-of-track threshold in ms for container tracks [default: 1000]
      --read-durations                 Read track durations metadata, then exit
      --scan-durations                 Scan all packets to compute per-track durations, then exit
      --decode-only <decode-only>      Decode, but do not play the audio
      --probe-only <probe-only>        Only probe the input for metadata
      --verify-only <verify-only>      Verify the decoded audio is valid, but do not play the audio
  -v, --verify <verify>                Verify the decoded audio is valid during playback
      --no-progress <no-progress>      Do not display playback progress
      --no-gapless <no-gapless>        Disable gapless decoding and playback
  -q, --quiet                          Suppress all console output
  -d <debug>                           Show debug output
  -h, --help                           Print help
  -V, --version                        Print version
```

**Notes**
- Logs are captured and shown in the TUI while the interface is active.
