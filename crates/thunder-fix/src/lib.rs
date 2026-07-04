use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use thunder_core::ThunderConfig;
pub use thunder_fix_rules::{Correction, ShellContext, suggest_with_rules};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixMode {
    Suggest,
    Apply,
}

pub fn suggest_fix(ctx: &ShellContext, config: &ThunderConfig) -> Result<Option<Correction>> {
    Ok(suggest_with_rules(ctx, config))
}

pub fn fix_command(mode: FixMode, config: &ThunderConfig, thefuck_path: Option<&str>) -> Result<String> {
    let ctx = ShellContext::from_env()?;

    if let Some(correction) = suggest_with_rules(&ctx, config) {
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
