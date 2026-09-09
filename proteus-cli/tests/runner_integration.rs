use std::process::Command;

#[cfg(feature = "effect-meter-cli")]
use tempfile::NamedTempFile;

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

#[test]
fn create_effects_json_command_succeeds() {
    let output = run_cli(&["create", "effects-json"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ConvolutionReverbSettings"));
}

#[test]
fn verify_probe_missing_input_returns_failure() {
    let output = run_cli(&["verify", "probe", "/definitely/missing.audio"]);
    assert!(!output.status.success());
}

#[test]
fn peaks_json_missing_input_returns_failure() {
    let output = run_cli(&["peaks", "json", "/definitely/missing.audio"]);
    assert!(!output.status.success());
}

#[cfg(not(feature = "effect-meter-cli"))]
#[test]
fn meter_effects_requires_feature_when_cli_harness_is_disabled() {
    let output = run_cli(&["meter", "effects", &fixture_path("test-16bit.wav")]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("effect-meter-cli"));
}

#[cfg(feature = "effect-meter-cli")]
#[test]
fn meter_effects_json_reports_gain_before_after_levels() {
    use proteus_lib::dsp::effects::{AudioEffect, GainEffect};

    let tmp = write_effects_json(vec![{
        let mut gain = GainEffect::default();
        gain.enabled = true;
        gain.settings.gain = 2.0;
        AudioEffect::Gain(gain)
    }]);

    let output = run_cli(&[
        "meter",
        "effects",
        &fixture_path("test-16bit.wav"),
        "--effects-json",
        tmp.path().to_str().expect("utf8 path"),
        "--duration",
        "0.25",
        "--format",
        "json",
        "--summary",
        "max",
    ]);
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("meter json report");
    let effects = report["effects"].as_array().expect("effects array");
    assert_eq!(effects.len(), 1);

    let input_peak = effects[0]["levels"]["input"]["peak"][0]
        .as_f64()
        .expect("input peak");
    let output_peak = effects[0]["levels"]["output"]["peak"][0]
        .as_f64()
        .expect("output peak");
    assert!(output_peak > input_peak);
}

#[cfg(feature = "effect-meter-cli")]
#[test]
fn meter_effects_table_prints_before_after_columns() {
    use proteus_lib::dsp::effects::{AudioEffect, GainEffect};

    let tmp = write_effects_json(vec![{
        let mut gain = GainEffect::default();
        gain.enabled = true;
        gain.settings.gain = 2.0;
        AudioEffect::Gain(gain)
    }]);

    let output = run_cli(&[
        "meter",
        "effects",
        &fixture_path("test-16bit.wav"),
        "--effects-json",
        tmp.path().to_str().expect("utf8 path"),
        "--duration",
        "0.25",
        "--format",
        "table",
    ]);
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("in_peak"));
    assert!(stdout.contains("out_peak"));
    assert!(stdout.contains("Gain"));
}

#[cfg(feature = "effect-meter-cli")]
#[test]
fn meter_effects_bars_prints_before_after_meter_lines() {
    use proteus_lib::dsp::effects::{AudioEffect, GainEffect};

    let tmp = write_effects_json(vec![{
        let mut gain = GainEffect::default();
        gain.enabled = true;
        gain.settings.gain = 2.0;
        AudioEffect::Gain(gain)
    }]);

    let output = run_cli(&[
        "meter",
        "effects",
        &fixture_path("test-16bit.wav"),
        "--effects-json",
        tmp.path().to_str().expect("utf8 path"),
        "--duration",
        "0.25",
        "--format",
        "bars",
    ]);
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[0] Gain"));
    assert!(stdout.contains("in :"));
    assert!(stdout.contains("out:"));
}

#[cfg(all(feature = "effect-meter-cli", feature = "effect-meter-cli-spectral"))]
#[test]
fn meter_effects_json_includes_spectral_section_when_requested() {
    use proteus_lib::dsp::effects::{AudioEffect, HighPassFilterEffect};

    let tmp = write_effects_json(vec![AudioEffect::HighPassFilter(
        HighPassFilterEffect::default(),
    )]);

    let output = run_cli(&[
        "meter",
        "effects",
        &fixture_path("test-16bit.wav"),
        "--effects-json",
        tmp.path().to_str().expect("utf8 path"),
        "--duration",
        "0.25",
        "--format",
        "json",
        "--spectral",
    ]);
    assert!(output.status.success());

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("meter json report");
    let spectral = report["spectral"].as_array().expect("spectral array");
    assert_eq!(spectral.len(), 1);
    assert!(spectral[0].is_object());
}

#[cfg(all(feature = "effect-meter-cli", feature = "effect-meter-cli-spectral"))]
#[test]
fn meter_effects_bars_renders_compact_spectral_graph_when_requested() {
    use proteus_lib::dsp::effects::{AudioEffect, LowPassFilterEffect};

    let tmp = write_effects_json(vec![AudioEffect::LowPassFilter(
        LowPassFilterEffect::default(),
    )]);

    let output = run_cli(&[
        "meter",
        "effects",
        &fixture_path("test-16bit.wav"),
        "--effects-json",
        tmp.path().to_str().expect("utf8 path"),
        "--duration",
        "0.25",
        "--format",
        "bars",
        "--spectral",
    ]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("spec o:"));
}

#[cfg(feature = "effect-meter-cli")]
fn write_effects_json(effects: Vec<proteus_lib::dsp::effects::AudioEffect>) -> NamedTempFile {
    let tmp = NamedTempFile::new().expect("temp file");
    std::fs::write(
        tmp.path(),
        serde_json::to_string_pretty(&effects).expect("serialize effects"),
    )
    .expect("write effects json");
    tmp
}
