use assert_cmd::Command;
use predicates::str::contains;

#[test]
fn create_effects_json_outputs_all_effects() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("prot"));
    cmd.args(["create", "effects-json"])
        .assert()
        .success()
        .stdout(contains("ConvolutionReverbSettings"))
        .stdout(contains("DelayReverbSettings"))
        .stdout(contains("LowPassFilterSettings"))
        .stdout(contains("HighPassFilterSettings"))
        .stdout(contains("DistortionSettings"))
        .stdout(contains("CompressorSettings"))
        .stdout(contains("LimiterSettings"))
        .stdout(contains("MultibandEqSettings"));
}
