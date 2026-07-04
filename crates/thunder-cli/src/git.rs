use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitState {
    Modified,
    Staged,
    Untracked,
}

pub fn git_status_map(cwd: &Path) -> HashMap<String, GitState> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(cwd)
        .output();

    let Ok(output) = output else {
        return HashMap::new();
    };
    if !output.status.success() {
        return HashMap::new();
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut map = HashMap::new();
    for line in text.lines() {
        if line.len() < 4 {
            continue;
        }
        let index = line.as_bytes()[0] as char;
        let worktree = line.as_bytes()[1] as char;
        let path = line[3..].trim();
        let path = path.split(" -> ").next().unwrap_or(path).trim();
        if index != ' ' && index != '?' {
            map.insert(path.to_string(), GitState::Staged);
        } else if worktree == '?' {
            map.insert(path.to_string(), GitState::Untracked);
        } else if worktree != ' ' {
            map.insert(path.to_string(), GitState::Modified);
        }
    }
    map
}

pub fn git_score_bonus(path: &str, git: &HashMap<String, GitState>) -> f64 {
    git.get(path)
        .or_else(|| git.iter().find(|(k, _)| path.ends_with(k.as_str())).map(|(_, v)| v))
        .map(|state| match state {
            GitState::Modified => 600.0,
            GitState::Staged => 500.0,
            GitState::Untracked => 200.0,
        })
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bonus_for_modified() {
        let mut map = HashMap::new();
        map.insert("src/lib.rs".into(), GitState::Modified);
        assert!(git_score_bonus("src/lib.rs", &map) > 500.0);
    }
}
