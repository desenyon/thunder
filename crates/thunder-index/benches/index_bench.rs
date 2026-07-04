use std::path::PathBuf;
use std::time::Instant;

use thunder_index::SearchIndex;

fn main() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    for i in 0..200 {
        let file = root.join(format!("file_{i}.txt"));
        let body = format!("line one {i}\nline two thunder {i}\n");
        std::fs::write(file, body).expect("write");
    }

    let mut index = SearchIndex::new(root.clone(), 2 * 1024 * 1024);
    let start = Instant::now();
    let lines = index.build().expect("build");
    let build_ms = start.elapsed().as_millis();

    let start = Instant::now();
    let hits = index.search("thunder", 100, true);
    let search_ms = start.elapsed().as_micros();

    println!("benchmark: {lines} lines indexed in {build_ms}ms, search {} hits in {search_ms}us", hits.len());
}
