use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn tn_help_works() {
    Command::cargo_bin("tn")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("s"));
}

#[test]
fn short_search_alias() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    Command::cargo_bin("tn")
        .unwrap()
        .current_dir(root)
        .args(["s", "--no-ui", "--no-daemon", "pick_from", "crates"])
        .assert()
        .success()
        .stdout(predicate::str::contains("thunder-pick"));
}
