use std::path::{Path, PathBuf};

use anyhow::Result;
use ignore::WalkBuilder;
use thunder_core::is_safe_relative_path;
use thunder_pick::{PickOptions, pick_lines};

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub display: String,
}

pub fn find_files(cwd: &Path, query: Option<&str>, limit: usize) -> Result<Vec<FileEntry>> {
    let mut walker = WalkBuilder::new(cwd);
    walker
        .hidden(false)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true);

    let mut entries = Vec::new();
    for entry in walker.build() {
        let entry = entry?;
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.into_path();
        let rel = path.strip_prefix(cwd).unwrap_or(&path);
        if !is_safe_relative_path(rel) {
            continue;
        }
        let display = rel.to_string_lossy().to_string();
        if let Some(query) = query {
            let q = query.to_lowercase();
            if !display.to_lowercase().contains(&q) {
                continue;
            }
        }
        entries.push(FileEntry { path, display });
        if entries.len() >= limit {
            break;
        }
    }
    Ok(entries)
}

pub fn pick_files(cwd: &Path, query: Option<&str>, pick: &PickOptions, limit: usize) -> Result<Vec<FileEntry>> {
    let files = find_files(cwd, query, limit)?;
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let labels: Vec<String> = files.iter().map(|f| f.display.clone()).collect();
    let selected = pick_lines(&labels, pick)?;
    let selected_set: std::collections::HashSet<&str> = selected.iter().map(String::as_str).collect();
    Ok(files
        .into_iter()
        .filter(|f| selected_set.contains(f.display.as_str()))
        .collect())
}
