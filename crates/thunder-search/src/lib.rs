use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use thunder_core::{ThunderConfig, validate_preview_command};
use thunder_index::{client_search, ensure_daemon};
use thunder_pick::{PickOptions, pick_lines};

#[derive(Debug, Clone)]
pub struct SearchMatch {
    pub path: PathBuf,
    pub line_number: u64,
    pub column: u64,
    pub line_text: String,
}

impl SearchMatch {
    pub fn display_line(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.path.display(),
            self.line_number,
            self.column,
            self.line_text.trim_end()
        )
    }

    pub fn path_line(&self) -> String {
        format!("{}:{}", self.path.display(), self.line_number)
    }
}

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub cwd: PathBuf,
    pub paths: Vec<PathBuf>,
    pub case_insensitive: bool,
    pub fixed_strings: bool,
    pub rg_path: Option<String>,
    pub pick: PickOptions,
    pub use_fzf: bool,
    pub preview_cmd: Option<String>,
    pub use_daemon: bool,
    pub config: ThunderConfig,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            paths: Vec::new(),
            case_insensitive: false,
            fixed_strings: false,
            rg_path: None,
            pick: PickOptions::default(),
            use_fzf: false,
            preview_cmd: None,
            use_daemon: true,
            config: ThunderConfig::default(),
        }
    }
}

impl SearchOptions {
    pub fn from_config(config: ThunderConfig) -> Self {
        let mut options = Self::default();
        options.use_daemon = config.search.use_daemon;
        options.use_fzf = config.pick.use_fzf;
        options.pick.height = config.pick.height.clone();
        options.preview_cmd = Some(config.pick.preview.clone());
        options.config = config;
        options
    }
}

/// Search with index or ripgrep and optionally open the skim/fzf picker.
pub fn search_interactive(query: &str, options: &SearchOptions) -> Result<Vec<SearchMatch>> {
    let matches = run_search(query, options)?;
    if matches.is_empty() {
        eprintln!("no matches for '{query}'");
        return Ok(matches);
    }

    let display_lines: Vec<String> = matches.iter().map(SearchMatch::display_line).collect();
    let mut pick_options = options.pick.clone();
    if pick_options.preview_cmd.is_none() {
        pick_options.preview_cmd = options
            .preview_cmd
            .clone()
            .or_else(default_preview_command);
    }
    if let Some(preview) = &pick_options.preview_cmd {
        validate_preview_command(preview)?;
    }

    let selected = if options.use_fzf {
        thunder_pick::pick_with_backend(&display_lines, &pick_options, true)?
    } else {
        pick_lines(&display_lines, &pick_options)?
    };

    Ok(map_selected_matches(&matches, &selected))
}

/// Search and print matches to stdout (no UI).
pub fn search_plain(query: &str, options: &SearchOptions) -> Result<Vec<SearchMatch>> {
    let matches = run_search(query, options)?;
    for m in &matches {
        println!("{}", m.display_line());
    }
    Ok(matches)
}

pub fn run_search(query: &str, options: &SearchOptions) -> Result<Vec<SearchMatch>> {
    if query.is_empty() {
        bail!("search query cannot be empty");
    }

    if options.use_daemon && options.config.search.use_daemon && is_literal_query(query, options) {
        if ensure_daemon(options.cwd.clone(), &options.config).is_ok() {
            if let Ok(matches) = search_via_daemon(query, options) {
                if !matches.is_empty() {
                    return Ok(matches);
                }
            }
        }
    }

    run_ripgrep(query, options)
}

fn is_literal_query(query: &str, options: &SearchOptions) -> bool {
    options.fixed_strings || !query.chars().any(|c| ".*+?[](){}^$\\|".contains(c))
}

fn search_via_daemon(query: &str, options: &SearchOptions) -> Result<Vec<SearchMatch>> {
    let hits = client_search(
        query,
        options.config.daemon.max_results,
        options.case_insensitive,
    )?;
    Ok(hits
        .into_iter()
        .map(|hit| SearchMatch {
            path: PathBuf::from(hit.path),
            line_number: hit.line_number,
            column: hit.column,
            line_text: hit.line_text,
        })
        .collect())
}

pub fn run_ripgrep(query: &str, options: &SearchOptions) -> Result<Vec<SearchMatch>> {
    let rg = resolve_rg(options.rg_path.as_deref())?;

    let mut command = Command::new(&rg);
    command
        .arg("--json")
        .arg("--line-number")
        .arg("--column")
        .arg("--no-heading")
        .arg("--color=never")
        .current_dir(&options.cwd);

    if options.case_insensitive {
        command.arg("-i");
    }
    if options.fixed_strings {
        command.arg("-F");
    }

    command.arg(query);
    if options.paths.is_empty() {
        command.arg(".");
    } else {
        for path in &options.paths {
            command.arg(path);
        }
    }

    command.stdout(Stdio::piped()).stderr(Stdio::null());

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn ripgrep at {}", rg.display()))?;

    let stdout = child
        .stdout
        .take()
        .context("ripgrep stdout was not captured")?;

    let reader = BufReader::new(stdout);
    let mut matches = Vec::new();

    for line in reader.lines() {
        let line = line.context("failed to read ripgrep output")?;
        if let Some(search_match) = parse_match_line(&line)? {
            matches.push(search_match);
        }
    }

    let status = child.wait().context("ripgrep exited unexpectedly")?;
    if !status.success() && status.code() != Some(1) {
        bail!("ripgrep failed with status {status}");
    }

    Ok(matches)
}

fn parse_match_line(line: &str) -> Result<Option<SearchMatch>> {
    let event: RgEvent = match serde_json::from_str(line) {
        Ok(event) => event,
        Err(_) => return Ok(None),
    };

    if event.kind != "match" {
        return Ok(None);
    }

    let data = event.data.context("ripgrep match event missing data")?;

    Ok(Some(SearchMatch {
        path: PathBuf::from(data.path.text),
        line_number: data.line_number,
        column: data.submatches.first().map(|s| s.start + 1).unwrap_or(1),
        line_text: data.lines.text,
    }))
}

fn map_selected_matches(all: &[SearchMatch], selected: &[String]) -> Vec<SearchMatch> {
    if selected.is_empty() {
        return Vec::new();
    }

    let selected_set: std::collections::HashSet<&str> =
        selected.iter().map(String::as_str).collect();

    all.iter()
        .filter(|m| selected_set.contains(m.display_line().as_str()))
        .cloned()
        .collect()
}

fn resolve_rg(explicit: Option<&str>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(PathBuf::from(path));
    }

    for candidate in ["rg", "/opt/homebrew/bin/rg", "/usr/local/bin/rg"] {
        if Command::new(candidate)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
        {
            return Ok(PathBuf::from(candidate));
        }
    }

    bail!("ripgrep (rg) not found in PATH; install ripgrep to use thunder search")
}

fn default_preview_command() -> Option<String> {
    Some("sed -n '{2}p' {1}".to_string())
}

#[derive(Debug, Deserialize)]
struct RgEvent {
    #[serde(rename = "type")]
    kind: String,
    data: Option<RgMatchData>,
}

#[derive(Debug, Deserialize)]
struct RgMatchData {
    path: RgText,
    lines: RgText,
    line_number: u64,
    submatches: Vec<RgSubmatch>,
}

#[derive(Debug, Deserialize)]
struct RgText {
    text: String,
}

#[derive(Debug, Deserialize)]
struct RgSubmatch {
    start: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ripgrep_match_json() {
        let line = r#"{"type":"match","data":{"path":{"text":"src/main.rs"},"lines":{"text":"fn main() {}"},"line_number":10,"absolute_offset":120,"submatches":[{"match":{"text":"main"},"start":3,"end":7}]}}"#;
        let parsed = parse_match_line(line).unwrap().unwrap();
        assert_eq!(parsed.path, PathBuf::from("src/main.rs"));
        assert_eq!(parsed.line_number, 10);
        assert_eq!(parsed.column, 4);
        assert_eq!(parsed.line_text, "fn main() {}");
    }

    #[test]
    fn literal_query_detection() {
        let options = SearchOptions::default();
        assert!(is_literal_query("hello", &options));
        assert!(!is_literal_query("he.llo", &options));
    }
}
