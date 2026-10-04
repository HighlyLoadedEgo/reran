use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn version_flag_works() {
    Command::cargo_bin("reran")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("reran 0.1.0"));
}
