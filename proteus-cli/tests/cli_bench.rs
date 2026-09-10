//! Black-box coverage for the processing benchmark report and terminal progress.
//!
//! These tests deliberately avoid asserting any duration or throughput values:
//! those are expected to vary across development and CI machines.  They verify
//! the report's scenarios and destination instead.

use std::process::Command;

use tempfile::tempdir;

fn run_cli(args: &[&str]) -> std::process::Output {
    let bin = env!("CARGO_BIN_EXE_prot");
    Command::new(bin).args(args).output().expect("run prot CLI")
}

fn fixture_path(name: &str) -> String {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../test_audio")
        .join(name)
        .to_string_lossy()
        .to_string()
}

const EVERY_EFFECT: &[&str] = &[
    "ConvolutionReverb",
    "DiffusionReverb",
    "DelayReverb",
    "LowPassFilter",
    "HighPassFilter",
    "Distortion",
    "Gain",
    "Compressor",
    "Limiter",
    "MultibandEq",
    "Pan",
];

fn assert_common_scenarios(report: &str) {
    assert!(
        report.contains("No effects"),
        "benchmark report must include the unprocessed baseline:\n{report}"
    );
    for effect in EVERY_EFFECT {
        assert!(
            report.contains(effect),
            "benchmark report must include {effect}:\n{report}"
        );
    }
}

#[test]
fn bench_audio_writes_readable_terminal_report_and_progress_by_default() {
    let input = fixture_path("test-16bit.wav");
    let output = run_cli(&[
        "bench",
        &input,
        "--iterations",
        "1",
        "--warmup-iterations",
        "0",
    ]);
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Case") && stdout.contains("Average"),
        "stdout should include a labelled terminal table:\n{stdout}"
    );
    assert!(
        !stdout.contains("# Proteus audio benchmark") && !stdout.contains("| ---"),
        "stdout must not use the Markdown report format:\n{stdout}"
    );
    assert_common_scenarios(&stdout);
    assert!(
        !stdout.contains("Project chain"),
        "ordinary audio must not claim a project chain:\n{stdout}"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Benchmarking") && stderr.contains("No effects"),
        "stderr should show benchmark-case progress:\n{stderr}"
    );
    assert!(
        !stderr.contains("[INFO]"),
        "benchmark progress must replace implementation-info logging:\n{stderr}"
    );
}

#[test]
fn bench_prot_includes_embedded_project_chain() {
    let input = fixture_path("demo-effects.prot");
    let output = run_cli(&[
        "bench",
        &input,
        "--iterations",
        "1",
        "--warmup-iterations",
        "0",
        "--chunk-frames",
        "16384",
    ]);
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_common_scenarios(&stdout);
    assert!(
        stdout.contains("Project chain"),
        ".prot benchmark must include its embedded chain:\n{stdout}"
    );
}

#[test]
fn bench_writes_markdown_to_requested_output_file_instead_of_stdout() {
    let directory = tempdir().expect("create output directory");
    let report_path = directory.path().join("benchmark.md");
    let input = fixture_path("test-16bit.wav");
    let output = run_cli(&[
        "bench",
        &input,
        "--iterations",
        "1",
        "--warmup-iterations",
        "0",
        "--output",
        report_path.to_str().expect("utf8 temporary path"),
    ]);
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("No effects"),
        "report should be redirected to the output file:\n{stdout}"
    );
    assert!(
        !stdout.contains("# Proteus audio benchmark"),
        "Markdown must only be written to the requested output file:\n{stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Benchmarking") && stderr.contains("No effects"),
        "saving a report should still show benchmark-case progress:\n{stderr}"
    );
    assert!(
        !stderr.contains("[INFO]"),
        "benchmark progress must replace implementation-info logging:\n{stderr}"
    );
    let report = std::fs::read_to_string(&report_path).expect("benchmark output file");
    assert!(report.contains("#"), "report should be Markdown:\n{report}");
    assert_common_scenarios(&report);
}
