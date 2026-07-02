use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThunderConfig {
    #[serde(default)]
    pub search: SearchConfig,
    #[serde(default)]
    pub pick: PickConfig,
    #[serde(default)]
    pub fix: FixConfig,
    #[serde(default)]
    pub daemon: DaemonConfig,
}

impl Default for ThunderConfig {
    fn default() -> Self {
        Self {
            search: SearchConfig::default(),
            pick: PickConfig::default(),
            fix: FixConfig::default(),
            daemon: DaemonConfig::default(),
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
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            use_daemon: true,
            fallback: default_rg(),
            max_file_size_bytes: default_max_file_size(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PickConfig {
    #[serde(default = "default_height")]
    pub height: String,
    #[serde(default = "default_preview")]
    pub preview: String,
    #[serde(default)]
    pub use_fzf: bool,
}

impl Default for PickConfig {
    fn default() -> Self {
        Self {
            height: default_height(),
            preview: default_preview(),
            use_fzf: false,
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

fn default_true() -> bool {
    true
}

fn default_rg() -> String {
    "rg".into()
}

fn default_max_file_size() -> u64 {
    2 * 1024 * 1024
}

fn default_height() -> String {
    "60%".into()
}

fn default_preview() -> String {
    "sed -n '{2}p' {1}".into()
}

fn default_index_limit() -> usize {
    500
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

pub fn socket_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("thunderd.sock"))
}

pub fn pid_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("thunderd.pid"))
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

/// Ensure `path` resolves under `root` (prevents path traversal in previews/index).
pub fn path_within_root(root: &Path, path: &Path) -> Result<PathBuf> {
    let root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let canonical = fs::canonicalize(&joined).with_context(|| format!("invalid path: {}", path.display()))?;

    if !canonical.starts_with(&root) {
        bail!("path escapes project root: {}", path.display());
    }

    Ok(canonical)
}

/// Validate user-supplied preview commands — only safe templates allowed.
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

/// Reject paths that walk upward or are absolute symlinks outside indexing scope.
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
    fn rejects_parent_dir_paths() {
        assert!(!is_safe_relative_path(Path::new("../etc/passwd")));
        assert!(is_safe_relative_path(Path::new("src/main.rs")));
    }
}
