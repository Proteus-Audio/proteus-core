use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn create_effects_json_outputs_all_effects() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("prot"));
    cmd.args(["create", "effects-json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ConvolutionReverbSettings"))
        .stdout(predicate::str::contains("DelayReverbSettings"))
        .stdout(predicate::str::contains("LowPassFilterSettings"))
        .stdout(predicate::str::contains("HighPassFilterSettings"))
        .stdout(predicate::str::contains("DistortionSettings"))
        .stdout(predicate::str::contains("CompressorSettings"))
        .stdout(predicate::str::contains("LimiterSettings"))
        .stdout(predicate::str::contains("MultibandEqSettings"));
}
