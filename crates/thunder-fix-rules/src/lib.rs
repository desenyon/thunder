use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
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

pub fn suggest_with_rules(
    ctx: &ShellContext,
    config: &ThunderConfig,
) -> Option<Correction> {
    for rule in enabled_rules(config) {
        if !rule.matches(ctx) {
            continue;
        }
        if let Some(correction) = rule.suggest(ctx) {
            return Some(correction);
        }
    }
    None
}

fn enabled_rules(config: &ThunderConfig) -> Vec<Box<dyn FixRule>> {
    let all = builtin_rules();
    if config.fix.enabled_rules.is_empty() {
        return all;
    }
    all.into_iter()
        .filter(|rule| config.fix.enabled_rules.iter().any(|n| n == rule.name()))
        .collect()
}

pub fn builtin_rules() -> Vec<Box<dyn FixRule>> {
    vec![
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
    ]
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
            && (ctx.command.starts_with("bew ") || ctx.command.starts_with("bre "))
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
        let correction = suggest_with_rules(&ctx, &ThunderConfig::default()).expect("correction");
        assert_eq!(correction.command, "git status");
    }
}
