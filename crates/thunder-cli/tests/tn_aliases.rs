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

#[test]
fn json_search_is_valid_without_no_ui() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let output = Command::cargo_bin("tn")
        .unwrap()
        .current_dir(root)
        .args(["s", "--json", "--no-daemon", "SearchOptions", "crates"])
        .output()
        .expect("run tn s --json");

    assert!(output.status.success(), "stderr={}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout must be a single JSON document");
    assert!(parsed.as_array().is_some_and(|a| !a.is_empty()));
    assert!(
        !stdout.contains("\n") || stdout.trim_end().ends_with(']'),
        "JSON output must not be followed by path:line text: {stdout}"
    );
}
