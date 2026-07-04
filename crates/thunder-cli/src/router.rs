use std::path::{Path, PathBuf};

use thunder_core::ThunderConfig;
use thunder_search::files::fuzzy_score;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryRoute {
    Search,
    Files,
    History,
}

pub fn route_query(query: &str, cwd: &Path, _config: &ThunderConfig) -> QueryRoute {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return QueryRoute::Search;
    }

    if looks_like_path(trimmed) || path_exists(cwd, trimmed) {
        return QueryRoute::Files;
    }

    if looks_like_shell_command(trimmed) {
        return QueryRoute::History;
    }

    if trimmed.contains('/') || trimmed.contains('.') && fuzzy_score(trimmed, trimmed) > 0.0 {
        return QueryRoute::Files;
    }

    QueryRoute::Search
}

fn looks_like_path(query: &str) -> bool {
    query.contains('/')
        || query.ends_with(".rs")
        || query.ends_with(".ts")
        || query.ends_with(".tsx")
        || query.ends_with(".js")
        || query.ends_with(".py")
        || query.ends_with(".go")
        || query.ends_with(".md")
        || query.ends_with(".toml")
        || query.ends_with(".json")
}

fn path_exists(cwd: &Path, query: &str) -> bool {
    let candidate = PathBuf::from(query);
    if candidate.is_absolute() {
        return candidate.exists();
    }
    cwd.join(query).exists()
}

fn looks_like_shell_command(query: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "git ", "cargo ", "npm ", "yarn ", "pnpm ", "docker ", "kubectl ", "make ",
        "python ", "python3 ", "go ", "rustc ", "tn ", "thunder ",
    ];
    PREFIXES.iter().any(|p| query.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_paths_to_files() {
        assert_eq!(route_query("src/main.rs", Path::new("."), &ThunderConfig::default()), QueryRoute::Files);
    }

    #[test]
    fn routes_commands_to_history() {
        assert_eq!(
            route_query("git status", Path::new("."), &ThunderConfig::default()),
            QueryRoute::History
        );
    }

    #[test]
    fn routes_terms_to_search() {
        assert_eq!(route_query("SearchIndex", Path::new("."), &ThunderConfig::default()), QueryRoute::Search);
    }
}
