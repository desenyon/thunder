use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

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
    #[serde(default)]
    pub theme: ThemeConfig,
    #[serde(default)]
    pub palette: PaletteConfig,
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
            theme: ThemeConfig::default(),
            palette: PaletteConfig::default(),
        }
    }
}

/// Palette plugin hooks and provider settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaletteConfig {
    /// Shell commands that emit `label|score|command` lines for the omni palette.
    #[serde(default)]
    pub plugin_commands: Vec<String>,
}

impl Default for PaletteConfig {
    fn default() -> Self {
        Self {
            plugin_commands: Vec::new(),
        }
    }
}

/// TUI color scheme. Defaults to high-contrast monochrome — no purple, no gradients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    /// skim color string: `bw` = black & white, `minimal` = custom monochrome accent
    #[serde(default = "default_theme_preset")]
    pub preset: String,
    /// Hide match-count / spinner info line in the picker
    #[serde(default = "default_true")]
    pub minimal_chrome: bool,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            preset: default_theme_preset(),
            minimal_chrome: true,
        }
    }
}

fn default_theme_preset() -> String {
    "minimal".into()
}

/// Resolve skim `--color` string from theme preset.
pub fn resolve_theme_color(config: &ThunderConfig) -> String {
    match config.theme.preset.as_str() {
        "bw" => "bw".into(),
        "dark" => "dark".into(),
        "minimal" | _ => {
            // Monochrome: white text, dark selection, cyan accent for matches only
            "fg:252,bg:235,matched:117,matched_bg:235,current:255,current_bg:238,\
             current_match:117,current_match_bg:238,prompt:252,cursor:255,selected:252,\
             border:240,header:245,info:240,spinner:240"
                .into()
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
    #[serde(default = "default_true")]
    pub streaming: bool,
    #[serde(default)]
    pub multi_select: bool,
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
            streaming: true,
            multi_select: false,
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
    /// Border between list and preview (`default`, `rounded`, `none`)
    #[serde(default = "default_layout")]
    pub layout: String,
}

fn default_layout() -> String {
    "default".into()
}

impl Default for PickConfig {
    fn default() -> Self {
        Self {
            height: default_height(),
            preview: None,
            use_fzf: false,
            reverse: true,
            prompt: default_prompt(),
            layout: default_layout(),
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
    #[serde(default = "default_recent_files_limit")]
    pub recent_files_limit: usize,
}

fn default_recent_files_limit() -> usize {
    100
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            max_entries: default_history_max(),
            palette_limit: default_palette_limit(),
            recent_files_limit: default_recent_files_limit(),
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

pub fn recent_files_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("recent.json"))
}

/// Frecency entry for smart palette ranking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentEntry {
    pub path: String,
    pub score: f64,
    pub last_access: u64,
}

pub fn record_recent_file(path: &Path, max_entries: usize) -> Result<()> {
    let path_str = path.to_string_lossy().to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut entries = load_recent_files(max_entries.saturating_mul(2))?;
    entries.retain(|e| e.path != path_str);

    let boosted = entries
        .iter()
        .map(|e| e.score)
        .fold(0.0_f64, f64::max)
        .max(1.0);
    entries.push(RecentEntry {
        path: path_str,
        score: boosted + 1.0,
        last_access: now,
    });

    entries.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    entries.truncate(max_entries);

    let dir = data_dir()?;
    fs::create_dir_all(&dir)?;
    fs::write(recent_files_path()?, serde_json::to_string_pretty(&entries)?)?;
    Ok(())
}

pub fn load_recent_files(limit: usize) -> Result<Vec<RecentEntry>> {
    let path = recent_files_path()?;
    let Ok(contents) = fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    let mut entries: Vec<RecentEntry> = serde_json::from_str(&contents).unwrap_or_default();
    entries.truncate(limit);
    Ok(entries)
}

pub fn recent_score_for(path: &str, entries: &[RecentEntry]) -> f64 {
    entries
        .iter()
        .find(|e| e.path == path || e.path.ends_with(path) || path.ends_with(&e.path))
        .map(|e| e.score)
        .unwrap_or(0.0)
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

pub fn tcp_port_path_for_root(root: &Path) -> Result<PathBuf> {
    Ok(data_dir()?.join(format!("thunderd-{}.port", hash_path(root)?)))
}

pub fn corpus_path_for_root(root: &Path) -> Result<PathBuf> {
    Ok(data_dir()?.join(format!("thunderd-{}.corpus", hash_path(root)?)))
}

pub fn pid_path_for_root(root: &Path) -> Result<PathBuf> {
    Ok(data_dir()?.join(format!("thunderd-{}.pid", hash_path(root)?)))
}

pub fn load_config() -> ThunderConfig {
    static CACHE: OnceLock<Mutex<(ThunderConfig, Option<SystemTime>)>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new((ThunderConfig::default(), None)));

    let path = match config_path() {
        Ok(path) => path,
        Err(_) => return ThunderConfig::default(),
    };

    let mtime = fs::metadata(&path).ok().and_then(|m| m.modified().ok());
    let mut guard = cache.lock().expect("config cache");
    if guard.1 == mtime {
        return guard.0.clone();
    }

    let Ok(contents) = fs::read_to_string(path) else {
        guard.0 = ThunderConfig::default();
        guard.1 = mtime;
        return guard.0.clone();
    };

    guard.0 = toml::from_str(&contents).unwrap_or_default();
    guard.1 = mtime;
    guard.0.clone()
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
    let _ = record_recent_file(path, config.history.recent_files_limit);
    let editor = resolve_editor(config);
    let mut parts = editor_argv(&editor)?;

    // Never route the path through a shell — pass argv so metacharacters in
    // filenames cannot inject commands (e.g. editor = "code --wait").
    let program = parts.remove(0);
    let mut command = Command::new(&program);
    command.args(&parts);
    if let Some(line) = line {
        command.arg(format!("+{line}"));
    }
    command.arg(path);

    let status = command
        .status()
        .with_context(|| format!("failed to launch editor: {editor}"))?;
    if !status.success() {
        bail!("editor exited with status {status}");
    }
    Ok(())
}

/// Split an editor command into argv without invoking a shell.
pub fn editor_argv(editor: &str) -> Result<Vec<String>> {
    let parts = shell_words::split(editor)
        .with_context(|| format!("failed to parse editor command: {editor}"))?;
    if parts.is_empty() {
        bail!("editor command is empty");
    }
    Ok(parts)
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
    fn editor_argv_splits_flags_without_shell() {
        let parts = editor_argv("code --wait").unwrap();
        assert_eq!(parts, vec!["code", "--wait"]);
    }

    #[test]
    fn editor_argv_preserves_quoted_paths() {
        let parts = editor_argv(r#""/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code" --wait"#)
            .unwrap();
        assert_eq!(
            parts[0],
            "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code"
        );
        assert_eq!(parts[1], "--wait");
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
