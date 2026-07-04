use std::process::Command;
use std::time::Duration;

use thunder_index::{client_ping, client_search, stop_daemon};

#[test]
fn daemon_indexes_and_searches_temp_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    std::fs::write(root.join("needle.txt"), "thunder integration test\n").unwrap();

    let _ = stop_daemon(&root);

    let thunderd = env!("CARGO_BIN_EXE_thunderd");
    let mut child = Command::new(thunderd)
        .arg("--root")
        .arg(&root)
        .spawn()
        .expect("spawn thunderd");

    let mut ready = false;
    for _ in 0..50 {
        if client_ping(&root).unwrap_or(false) {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(ready, "daemon did not become ready");

    let hits = client_search(&root, "thunder", 10, true).expect("search");
    assert!(!hits.is_empty(), "expected search hits");

    child.kill().ok();
    let _ = stop_daemon(&root);
}
