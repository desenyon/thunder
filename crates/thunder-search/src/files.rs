use std::path::{Path, PathBuf};

use anyhow::Result;
use ignore::WalkBuilder;
use thunder_core::is_safe_relative_path;
use thunder_index::{client_list_files, client_ping, ensure_daemon};
use thunder_pick::{PickOptions, pick_lines};

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub display: String,
    pub score: f64,
}

/// Fuzzy subsequence score — higher is better. Returns 0.0 when no match.
pub fn fuzzy_score(haystack: &str, needle: &str) -> f64 {
    if needle.is_empty() {
        return 1.0;
    }
    let h = haystack.to_lowercase();
    let n = needle.to_lowercase();
    if h == n {
        return 1000.0;
    }
    if h.contains(&n) {
        return 500.0 + (n.len() as f64 / h.len() as f64) * 100.0;
    }

    let h_chars: Vec<char> = h.chars().collect();
    let n_chars: Vec<char> = n.chars().collect();
    let mut hi = 0;
    let mut consecutive = 0;
    let mut max_consecutive = 0;
    for &nc in &n_chars {
        let mut found = false;
        while hi < h_chars.len() {
            if h_chars[hi] == nc {
                consecutive += 1;
                max_consecutive = max_consecutive.max(consecutive);
                hi += 1;
                found = true;
                break;
            }
            consecutive = 0;
            hi += 1;
        }
        if !found {
            return 0.0;
        }
    }

    let base = (n_chars.len() as f64 / h_chars.len() as f64) * 200.0;
    base + max_consecutive as f64 * 10.0
}

pub fn find_files(cwd: &Path, query: Option<&str>, limit: usize) -> Result<Vec<FileEntry>> {
    let root = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());

    if client_ping(&root).unwrap_or(false) {
        let prefix = query.filter(|q| !q.is_empty());
        if let Ok(paths) = client_list_files(&root, prefix, limit.saturating_mul(4)) {
            if !paths.is_empty() {
                return Ok(rank_file_paths(&root, paths, query, limit));
            }
        }
    }

    find_files_walk(&root, query, limit)
}

fn rank_file_paths(
    root: &Path,
    paths: Vec<String>,
    query: Option<&str>,
    limit: usize,
) -> Vec<FileEntry> {
    let mut entries: Vec<FileEntry> = paths
        .into_iter()
        .filter_map(|display| {
            if !is_safe_relative_path(Path::new(&display)) {
                return None;
            }
            let score = query
                .map(|q| fuzzy_score(&display, q))
                .unwrap_or(1.0);
            if query.is_some() && score <= 0.0 {
                return None;
            }
            Some(FileEntry {
                path: root.join(&display),
                display,
                score,
            })
        })
        .collect();

    entries.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.display.cmp(&b.display))
    });
    entries.truncate(limit);
    entries
}

fn find_files_walk(cwd: &Path, query: Option<&str>, limit: usize) -> Result<Vec<FileEntry>> {
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
        let score = query
            .map(|q| fuzzy_score(&display, q))
            .unwrap_or(1.0);
        if query.is_some() && score <= 0.0 {
            continue;
        }
        entries.push(FileEntry {
            path,
            display,
            score,
        });
        if entries.len() >= limit.saturating_mul(4) {
            break;
        }
    }

    entries.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.display.cmp(&b.display))
    });
    entries.truncate(limit);
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

pub fn warm_file_index(cwd: &Path, config: &thunder_core::ThunderConfig) -> Result<()> {
    let root = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    if config.search.use_daemon {
        let _ = ensure_daemon(root, config);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_prefers_substring() {
        assert!(fuzzy_score("src/lib.rs", "lib") > fuzzy_score("src/main.rs", "lib"));
        assert_eq!(fuzzy_score("foo", "bar"), 0.0);
    }

    #[test]
    fn fuzzy_exact_match_wins() {
        assert!(fuzzy_score("auth.rs", "auth.rs") > fuzzy_score("auth_module.rs", "auth"));
    }
}
