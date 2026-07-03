use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use thunder_core::ThunderConfig;

#[derive(Debug, Clone)]
pub struct ShellContext {
    pub command: String,
    pub exit_code: i32,
    pub stderr: String,
    pub cwd: PathBuf,
}

impl ShellContext {
    pub fn from_env() -> Result<Self> {
        let command = std::env::var("THUNDER_LAST_CMD").unwrap_or_default();
        if command.is_empty() {
            bail!("no previous command recorded; source thunder shell integration");
        }

        let exit_code = std::env::var("THUNDER_LAST_EXIT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        let stderr = std::env::var("THUNDER_LAST_STDERR").unwrap_or_default();
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        Ok(Self {
            command,
            exit_code,
            stderr,
            cwd,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixMode {
    Suggest,
    Apply,
}

#[derive(Debug, Clone)]
pub struct Correction {
    pub command: String,
    pub rule: String,
}

pub trait FixRule: Send + Sync {
    fn name(&self) -> &'static str;
    fn matches(&self, ctx: &ShellContext) -> bool;
    fn suggest(&self, ctx: &ShellContext) -> Option<Correction>;
}

pub fn suggest_fix(ctx: &ShellContext, config: &ThunderConfig) -> Result<Option<Correction>> {
    let rules = enabled_rules(config);
    for rule in rules {
        if !rule.matches(ctx) {
            continue;
        }
        if let Some(correction) = rule.suggest(ctx) {
            return Ok(Some(correction));
        }
    }
    Ok(None)
}

pub fn fix_command(mode: FixMode, config: &ThunderConfig, thefuck_path: Option<&str>) -> Result<String> {
    let ctx = ShellContext::from_env()?;

    if let Some(correction) = suggest_fix(&ctx, config)? {
        if mode == FixMode::Apply {
            apply_correction(&correction.command)?;
        }
        return Ok(correction.command);
    }

    if config.fix.use_thefuck_fallback {
        return fix_via_thefuck(mode, thefuck_path);
    }

    bail!("no correction found for: {}", ctx.command)
}

fn apply_correction(command: &str) -> Result<()> {
    validate_safe_command(command)?;
    let status = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to run correction")?;

    if !status.success() {
        bail!("correction command failed with status {status}");
    }
    Ok(())
}

/// Block obviously dangerous corrections before execution.
pub fn validate_safe_command(command: &str) -> Result<()> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        bail!("empty command");
    }

    if trimmed.contains('\n') || trimmed.contains('\r') {
        bail!("multiline commands are not allowed");
    }

    const BLOCKED: &[&str] = &["rm -rf /", ":(){ :|:& };:", "mkfs", "dd if=", "> /dev/sd"];
    for pattern in BLOCKED {
        if trimmed.contains(pattern) {
            bail!("blocked unsafe correction");
        }
    }

    Ok(())
}

fn enabled_rules(config: &ThunderConfig) -> Vec<Box<dyn FixRule>> {
    let all: Vec<Box<dyn FixRule>> = vec![
        Box::new(GitTypoRule),
        Box::new(SudoRule),
        Box::new(CdTypoRule),
        Box::new(NpmRule),
        Box::new(DockerRule),
        Box::new(ManRule),
        Box::new(PythonRule),
        Box::new(CargoRule),
        Box::new(PipRule),
        Box::new(KubectlRule),
        Box::new(BrewRule),
    ];

    if config.fix.enabled_rules.is_empty() {
        return all;
    }

    all.into_iter()
        .filter(|rule| config.fix.enabled_rules.iter().any(|n| n == rule.name()))
        .collect()
}

struct GitTypoRule;

impl FixRule for GitTypoRule {
    fn name(&self) -> &'static str {
        "git"
    }

    fn matches(&self, ctx: &ShellContext) -> bool {
        ctx.exit_code != 0
            && (ctx.command.starts_with("gti ")
                || ctx.command.starts_with("gt ")
                || ctx.stderr.contains("git: command not found"))
    }

    fn suggest(&self, ctx: &ShellContext) -> Option<Correction> {
        let fixed = ctx
            .command
            .replacen("gti ", "git ", 1)
            .replacen("gt ", "git ", 1);
        if fixed != ctx.command {
            Some(Correction {
                command: fixed,
                rule: self.name().into(),
            })
        } else if ctx.stderr.contains("git: command not found") {
            Some(Correction {
                command: "brew install git".into(),
                rule: self.name().into(),
            })
        } else {
            None
        }
    }
}

struct SudoRule;

impl FixRule for SudoRule {
    fn name(&self) -> &'static str {
        "sudo"
    }

    fn matches(&self, ctx: &ShellContext) -> bool {
        ctx.exit_code != 0
            && (ctx.stderr.contains("Permission denied")
                || ctx.stderr.contains("Operation not permitted"))
            && !ctx.command.starts_with("sudo ")
    }

    fn suggest(&self, ctx: &ShellContext) -> Option<Correction> {
        Some(Correction {
            command: format!("sudo {}", ctx.command),
            rule: self.name().into(),
        })
    }
}

struct CdTypoRule;

impl FixRule for CdTypoRule {
    fn name(&self) -> &'static str {
        "cd"
    }

    fn matches(&self, ctx: &ShellContext) -> bool {
        ctx.exit_code != 0
            && ctx.command.starts_with("cd ")
            && (ctx.stderr.contains("no such file or directory")
                || ctx.stderr.contains("No such file or directory"))
    }

    fn suggest(&self, ctx: &ShellContext) -> Option<Correction> {
        let target = ctx.command.trim_start_matches("cd ").trim();
        let best = find_closest_dir(&ctx.cwd, target)?;
        Some(Correction {
            command: format!("cd {best}"),
            rule: self.name().into(),
        })
    }
}

fn find_closest_dir(cwd: &Path, typo: &str) -> Option<String> {
    let typo_lower = typo.to_lowercase();
    let mut best: Option<(usize, String)> = None;

    let read_dir = fs::read_dir(cwd).ok()?;
    for entry in read_dir.flatten() {
        if !entry.file_type().ok()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let dist = levenshtein(&typo_lower, &name.to_lowercase());
        if dist <= 3 {
            if best.as_ref().is_none_or(|(d, _)| dist < *d) {
                best = Some((dist, name));
            }
        }
    }

    best.map(|(_, name)| name)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur.push((prev[j] + cost).min(cur[j] + 1).min(prev[j + 1] + 1));
        }
        prev = cur;
    }
    *prev.last().unwrap_or(&usize::MAX)
}

struct NpmRule;

impl FixRule for NpmRule {
    fn name(&self) -> &'static str {
        "npm"
    }

    fn matches(&self, ctx: &ShellContext) -> bool {
        ctx.exit_code != 0
            && (ctx.command.starts_with("pm ") || ctx.command.starts_with("nom "))
    }

    fn suggest(&self, ctx: &ShellContext) -> Option<Correction> {
        let fixed = ctx
            .command
            .replacen("pm ", "npm ", 1)
            .replacen("nom ", "npm ", 1);
        Some(Correction {
            command: fixed,
            rule: self.name().into(),
        })
    }
}

struct DockerRule;

impl FixRule for DockerRule {
    fn name(&self) -> &'static str {
        "docker"
    }

    fn matches(&self, ctx: &ShellContext) -> bool {
        ctx.exit_code != 0 && ctx.command.starts_with("doker ")
    }

    fn suggest(&self, ctx: &ShellContext) -> Option<Correction> {
        Some(Correction {
            command: ctx.command.replacen("doker ", "docker ", 1),
            rule: self.name().into(),
        })
    }
}

struct ManRule;

impl FixRule for ManRule {
    fn name(&self) -> &'static str {
        "man"
    }

    fn matches(&self, ctx: &ShellContext) -> bool {
        ctx.exit_code != 0
            && ctx.command.starts_with("man ")
            && ctx.stderr.contains("No manual entry")
    }

    fn suggest(&self, ctx: &ShellContext) -> Option<Correction> {
        let topic = ctx.command.trim_start_matches("man ").trim();
        Some(Correction {
            command: format!("help {topic}"),
            rule: self.name().into(),
        })
    }
}

struct PythonRule;

impl FixRule for PythonRule {
    fn name(&self) -> &'static str {
        "python"
    }

    fn matches(&self, ctx: &ShellContext) -> bool {
        ctx.exit_code != 0
            && (ctx.command.starts_with("pyhton ")
                || ctx.command.starts_with("pyton ")
                || ctx.stderr.contains("python: command not found"))
    }

    fn suggest(&self, ctx: &ShellContext) -> Option<Correction> {
        if ctx.stderr.contains("python: command not found") {
            return Some(Correction {
                command: ctx.command.replacen("python", "python3", 1),
                rule: self.name().into(),
            });
        }
        let fixed = ctx
            .command
            .replacen("pyhton ", "python3 ", 1)
            .replacen("pyton ", "python3 ", 1);
        Some(Correction {
            command: fixed,
            rule: self.name().into(),
        })
    }
}

struct CargoRule;

impl FixRule for CargoRule {
    fn name(&self) -> &'static str {
        "cargo"
    }

    fn matches(&self, ctx: &ShellContext) -> bool {
        ctx.exit_code != 0
            && (ctx.command.starts_with("cago ")
                || ctx.command.starts_with("crgo ")
                || ctx.stderr.contains("cargo: command not found"))
    }

    fn suggest(&self, ctx: &ShellContext) -> Option<Correction> {
        if ctx.stderr.contains("cargo: command not found") {
            return Some(Correction {
                command: "rustup update stable && cargo --version".into(),
                rule: self.name().into(),
            });
        }
        let fixed = ctx
            .command
            .replacen("cago ", "cargo ", 1)
            .replacen("crgo ", "cargo ", 1);
        Some(Correction {
            command: fixed,
            rule: self.name().into(),
        })
    }
}

struct PipRule;

impl FixRule for PipRule {
    fn name(&self) -> &'static str {
        "pip"
    }

    fn matches(&self, ctx: &ShellContext) -> bool {
        ctx.exit_code != 0 && (ctx.command.starts_with("pi ") || ctx.command.starts_with("pp "))
    }

    fn suggest(&self, ctx: &ShellContext) -> Option<Correction> {
        let fixed = ctx
            .command
            .replacen("pi ", "pip3 ", 1)
            .replacen("pp ", "pip3 ", 1);
        Some(Correction {
            command: fixed,
            rule: self.name().into(),
        })
    }
}

struct KubectlRule;

impl FixRule for KubectlRule {
    fn name(&self) -> &'static str {
        "kubectl"
    }

    fn matches(&self, ctx: &ShellContext) -> bool {
        ctx.exit_code != 0
            && (ctx.command.starts_with("kubctl ")
                || ctx.command.starts_with("kubeclt ")
                || ctx.command.starts_with("kbectl "))
    }

    fn suggest(&self, ctx: &ShellContext) -> Option<Correction> {
        let fixed = ctx
            .command
            .replacen("kubctl ", "kubectl ", 1)
            .replacen("kubeclt ", "kubectl ", 1)
            .replacen("kbectl ", "kubectl ", 1);
        Some(Correction {
            command: fixed,
            rule: self.name().into(),
        })
    }
}

struct BrewRule;

impl FixRule for BrewRule {
    fn name(&self) -> &'static str {
        "brew"
    }

    fn matches(&self, ctx: &ShellContext) -> bool {
        ctx.exit_code != 0
            && (ctx.command.starts_with("bew ")
                || ctx.command.starts_with("bre "))
    }

    fn suggest(&self, ctx: &ShellContext) -> Option<Correction> {
        let fixed = ctx
            .command
            .replacen("bew ", "brew ", 1)
            .replacen("bre ", "brew ", 1);
        Some(Correction {
            command: fixed,
            rule: self.name().into(),
        })
    }
}

fn fix_via_thefuck(mode: FixMode, thefuck_path: Option<&str>) -> Result<String> {
    let thefuck = resolve_thefuck(thefuck_path)?;

    let mut command = Command::new(&thefuck);
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if mode == FixMode::Apply {
        command.arg("--yeah");
    }

    let output = command
        .output()
        .with_context(|| format!("failed to run thefuck at {}", thefuck.display()))?;

    if !output.status.success() {
        bail!("thefuck did not find a correction");
    }

    let suggestion = String::from_utf8(output.stdout).context("thefuck output was not valid utf-8")?;
    let suggestion = suggestion.trim().to_string();
    if suggestion.is_empty() {
        bail!("thefuck did not return a correction");
    }
    Ok(suggestion)
}

fn resolve_thefuck(explicit: Option<&str>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(PathBuf::from(path));
    }

    for candidate in [
        "thefuck",
        "/opt/homebrew/bin/thefuck",
        "/usr/local/bin/thefuck",
    ] {
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

    bail!("thefuck not found in PATH; install thefuck or disable fallback in config")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixes_git_typo() {
        let ctx = ShellContext {
            command: "gti status".into(),
            exit_code: 127,
            stderr: String::new(),
            cwd: PathBuf::from("."),
        };
        let correction = suggest_fix(&ctx, &ThunderConfig::default()).unwrap().unwrap();
        assert_eq!(correction.command, "git status");
    }

    #[test]
    fn blocks_dangerous_apply() {
        assert!(validate_safe_command("rm -rf /").is_err());
        assert!(validate_safe_command("git status").is_ok());
    }

    #[test]
    fn suggests_sudo() {
        let ctx = ShellContext {
            command: "apt install vim".into(),
            exit_code: 1,
            stderr: "Permission denied".into(),
            cwd: PathBuf::from("."),
        };
        let correction = suggest_fix(&ctx, &ThunderConfig::default()).unwrap().unwrap();
        assert_eq!(correction.command, "sudo apt install vim");
    }
}
