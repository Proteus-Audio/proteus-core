use std::process::Command;

fn run_cli(args: &[&str]) -> std::process::Output {
    let bin = env!("CARGO_BIN_EXE_prot");
    Command::new(bin).args(args).output().expect("run prot CLI")
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
