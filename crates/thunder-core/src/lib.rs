use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThunderConfig {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub search: SearchConfig,
    #[serde(default)]
    pub pick: PickConfig,
    #[serde(default)]
    pub fix: FixConfig,
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub history: HistoryConfig,
}

impl Default for ThunderConfig {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            search: SearchConfig::default(),
            pick: PickConfig::default(),
            fix: FixConfig::default(),
            daemon: DaemonConfig::default(),
            history: HistoryConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    #[serde(default)]
    pub editor: Option<String>,
    #[serde(default = "default_true")]
    pub open_on_select: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            editor: None,
            open_on_select: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    #[serde(default = "default_true")]
    pub use_daemon: bool,
    #[serde(default = "default_rg")]
    pub fallback: String,
    #[serde(default = "default_max_file_size")]
    pub max_file_size_bytes: u64,
    #[serde(default = "default_max_results")]
    pub max_results: usize,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            use_daemon: true,
            fallback: default_rg(),
            max_file_size_bytes: default_max_file_size(),
            max_results: default_max_results(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PickConfig {
    #[serde(default = "default_height")]
    pub height: String,
    #[serde(default)]
    pub preview: Option<String>,
    #[serde(default)]
    pub use_fzf: bool,
    #[serde(default = "default_true")]
    pub reverse: bool,
    #[serde(default = "default_prompt")]
    pub prompt: String,
}

impl Default for PickConfig {
    fn default() -> Self {
        Self {
            height: default_height(),
            preview: None,
            use_fzf: false,
            reverse: true,
            prompt: default_prompt(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixConfig {
    #[serde(default = "default_true")]
    pub use_thefuck_fallback: bool,
    #[serde(default)]
    pub enabled_rules: Vec<String>,
}

impl Default for FixConfig {
    fn default() -> Self {
        Self {
            use_thefuck_fallback: true,
            enabled_rules: vec![
                "git".into(),
                "sudo".into(),
                "cd".into(),
                "npm".into(),
                "docker".into(),
                "man".into(),
                "python".into(),
                "cargo".into(),
                "pip".into(),
                "kubectl".into(),
                "brew".into(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default = "default_true")]
    pub auto_start: bool,
    #[serde(default = "default_index_limit")]
    pub max_results: usize,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            auto_start: true,
            max_results: default_index_limit(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryConfig {
    #[serde(default = "default_history_max")]
    pub max_entries: usize,
    #[serde(default = "default_palette_limit")]
    pub palette_limit: usize,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            max_entries: default_history_max(),
            palette_limit: default_palette_limit(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_rg() -> String {
    "rg".into()
}

fn default_max_file_size() -> u64 {
    2 * 1024 * 1024
}

fn default_max_results() -> usize {
    500
}

fn default_height() -> String {
    "60%".into()
}

fn default_prompt() -> String {
    "> ".into()
}

fn default_index_limit() -> usize {
    500
}

fn default_history_max() -> usize {
    2000
}

fn default_palette_limit() -> usize {
    200
}

pub fn config_dir() -> Result<PathBuf> {
    dirs::config_dir()
        .map(|d| d.join("thunder"))
        .context("could not resolve config directory")
}

pub fn data_dir() -> Result<PathBuf> {
    dirs::data_local_dir()
        .map(|d| d.join("thunder"))
        .context("could not resolve data directory")
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

pub fn history_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("history"))
}

pub fn stderr_cache_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("last.stderr"))
}

fn hash_path(path: &Path) -> Result<String> {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    canonical.hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}

pub fn socket_path_for_root(root: &Path) -> Result<PathBuf> {
    Ok(data_dir()?.join(format!("thunderd-{}.sock", hash_path(root)?)))
}

pub fn pid_path_for_root(root: &Path) -> Result<PathBuf> {
    Ok(data_dir()?.join(format!("thunderd-{}.pid", hash_path(root)?)))
}

pub fn load_config() -> ThunderConfig {
    let path = match config_path() {
        Ok(path) => path,
        Err(_) => return ThunderConfig::default(),
    };

    let Ok(contents) = fs::read_to_string(path) else {
        return ThunderConfig::default();
    };

    toml::from_str(&contents).unwrap_or_default()
}

pub fn save_config(config: &ThunderConfig) -> Result<()> {
    let dir = config_dir()?;
    fs::create_dir_all(&dir).context("failed to create config directory")?;
    let contents = toml::to_string_pretty(config).context("failed to serialize config")?;
    fs::write(config_path()?, contents).context("failed to write config")?;
    Ok(())
}

pub fn resolve_preview(config: &ThunderConfig) -> String {
    if let Some(preview) = &config.pick.preview {
        return preview.clone();
    }
    detect_preview_command()
}

pub fn detect_preview_command() -> String {
    for candidate in ["bat --color=always --line-range={2}:{2} {1}", "bat -n --color=always {1}"] {
        let program = candidate.split_whitespace().next().unwrap_or("");
        if Command::new(program)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
        {
            return candidate.to_string();
        }
    }
    "sed -n '{2}p' {1}".into()
}

pub fn resolve_editor(config: &ThunderConfig) -> String {
    if let Some(editor) = &config.general.editor {
        return editor.clone();
    }
    for key in ["THUNDER_EDITOR", "VISUAL", "EDITOR"] {
        if let Ok(value) = std::env::var(key) {
            if !value.trim().is_empty() {
                return value;
            }
        }
    }
    "vi".into()
}

pub fn open_in_editor(config: &ThunderConfig, path: &Path, line: Option<u64>) -> Result<()> {
    let editor = resolve_editor(config);
    let mut command = if editor.contains('/') || editor.contains(' ') {
        Command::new("sh")
    } else {
        Command::new(&editor)
    };

    if editor.contains('/') || editor.contains(' ') {
        let script = if let Some(line) = line {
            format!("{editor} +{line} {}", path.display())
        } else {
            format!("{editor} {}", path.display())
        };
        command.arg("-c").arg(script);
    } else if let Some(line) = line {
        command.arg(format!("+{line}")).arg(path);
    } else {
        command.arg(path);
    }

    let status = command
        .status()
        .with_context(|| format!("failed to launch editor: {editor}"))?;
    if !status.success() {
        bail!("editor exited with status {status}");
    }
    Ok(())
}

pub fn resolve_binary(configured: &str, fallbacks: &[&str]) -> Option<PathBuf> {
    let candidates = std::iter::once(configured)
        .chain(fallbacks.iter().copied())
        .collect::<Vec<_>>();
    for candidate in candidates {
        if candidate.contains('/') {
            let path = PathBuf::from(candidate);
            if path.is_file() {
                return Some(path);
            }
        } else if Command::new(candidate)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
        {
            return Some(PathBuf::from(candidate));
        }
    }
    None
}

pub fn append_history(command: &str, max_entries: usize) -> Result<()> {
    let trimmed = command.trim();
    if trimmed.is_empty()
        || trimmed == "tn"
        || trimmed == "thunder"
        || trimmed.starts_with("tn ")
        || trimmed.starts_with("thunder ")
    {
        return Ok(());
    }

    let path = history_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut lines = read_history_lines(usize::MAX)?;
    lines.retain(|line| line != trimmed);
    lines.push(trimmed.to_string());
    if lines.len() > max_entries {
        let drain = lines.len() - max_entries;
        lines.drain(0..drain);
    }

    let mut file = fs::File::create(&path)?;
    for line in lines {
        writeln!(file, "{line}")?;
    }
    Ok(())
}

pub fn read_history_lines(limit: usize) -> Result<Vec<String>> {
    let path = history_path()?;
    let Ok(contents) = fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    let mut lines: Vec<String> = contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    if lines.len() > limit {
        lines = lines.split_off(lines.len() - limit);
    }
    Ok(lines)
}

pub fn path_within_root(root: &Path, path: &Path) -> Result<PathBuf> {
    let root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let canonical =
        fs::canonicalize(&joined).with_context(|| format!("invalid path: {}", path.display()))?;

    if !canonical.starts_with(&root) {
        bail!("path escapes project root: {}", path.display());
    }

    Ok(canonical)
}

pub fn validate_preview_command(cmd: &str) -> Result<()> {
    if cmd.is_empty() {
        bail!("preview command cannot be empty");
    }

    const FORBIDDEN: &[&str] = &[";", "|", "&", "$", "`", "$(", "\n", "\r"];
    for token in FORBIDDEN {
        if cmd.contains(token) {
            bail!("preview command contains unsafe shell token: {token}");
        }
    }

    if !cmd.contains("{1}") {
        bail!("preview command must include {{1}} placeholder for file path");
    }

    Ok(())
}

pub fn is_safe_relative_path(path: &Path) -> bool {
    !path.components().any(|c| matches!(c, Component::ParentDir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_preview() {
        assert!(validate_preview_command("cat {1}; rm -rf /").is_err());
        assert!(validate_preview_command("sed -n '{2}p' {1}").is_ok());
    }

    #[test]
    fn history_dedupes() {
        let temp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("XDG_DATA_HOME", temp.path()) };

        append_history("git status", 10).unwrap();
        append_history("cargo build", 10).unwrap();
        append_history("git status", 10).unwrap();

        let lines = read_history_lines(10).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "cargo build");
        assert_eq!(lines[1], "git status");
    }
}
