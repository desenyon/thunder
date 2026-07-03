use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use ignore::WalkBuilder;
use thunder_core::{
    ThunderConfig, open_in_editor, read_history_lines, resolve_preview, validate_preview_command,
};
use thunder_fix::validate_safe_command;
use thunder_pick::{PickOptions, pick_lines};

#[derive(Debug, Clone)]
struct PaletteItem {
    label: String,
    action: PaletteAction,
}

#[derive(Debug, Clone)]
enum PaletteAction {
    OpenFile(PathBuf),
    RerunCommand(String),
}

pub fn run_palette(config: &ThunderConfig, cwd: &Path, execute: bool) -> Result<()> {
    let mut items = Vec::new();
    items.extend(collect_history_items(config)?);
    items.extend(collect_file_items(cwd)?);

    if items.is_empty() {
        bail!("palette is empty; run some commands or add files in this directory");
    }

    let labels: Vec<String> = items.iter().map(|i| i.label.clone()).collect();
    let preview = resolve_preview(config);
    validate_preview_command(&preview)?;

    let pick_options = PickOptions {
        height: config.pick.height.clone(),
        reverse: config.pick.reverse,
        prompt: "thunder> ".into(),
        preview_cmd: Some(preview),
        ..PickOptions::default()
    };

    let selected = pick_lines(&labels, &pick_options)?;
    let Some(label) = selected.into_iter().next() else {
        return Ok(());
    };

    let item = items
        .into_iter()
        .find(|i| i.label == label)
        .context("selected palette item disappeared")?;

    execute_action(item.action, config, cwd, execute)
}

fn collect_history_items(config: &ThunderConfig) -> Result<Vec<PaletteItem>> {
    let lines = read_history_lines(config.history.palette_limit)?;
    let mut items = Vec::new();
    for line in lines.into_iter().rev() {
        items.push(PaletteItem {
            label: format!("[hist] {line}"),
            action: PaletteAction::RerunCommand(line),
        });
    }
    Ok(items)
}

fn collect_file_items(cwd: &Path) -> Result<Vec<PaletteItem>> {
    let mut walker = WalkBuilder::new(cwd);
    walker
        .hidden(false)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .max_depth(Some(8));
    let walker = walker.build();

    let mut items = Vec::new();
    for entry in walker {
        let entry = entry.context("walk failed")?;
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.into_path();
        let rel = path
            .strip_prefix(cwd)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        items.push(PaletteItem {
            label: format!("[file] {rel}"),
            action: PaletteAction::OpenFile(path),
        });
        if items.len() >= 500 {
            break;
        }
    }
    Ok(items)
}

fn execute_action(
    action: PaletteAction,
    config: &ThunderConfig,
    cwd: &Path,
    execute: bool,
) -> Result<()> {
    match action {
        PaletteAction::OpenFile(path) => {
            let full = if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            };
            if execute || config.general.open_on_select {
                open_in_editor(config, &full, Some(1))?;
            } else {
                println!("{}", full.strip_prefix(cwd).unwrap_or(&full).display());
            }
        }
        PaletteAction::RerunCommand(cmd) => {
            validate_safe_command(&cmd)?;
            if execute {
                let status = Command::new("sh")
                    .arg("-c")
                    .arg(&cmd)
                    .stdin(std::process::Stdio::inherit())
                    .stdout(std::process::Stdio::inherit())
                    .stderr(std::process::Stdio::inherit())
                    .status()
                    .context("failed to rerun command")?;
                if !status.success() {
                    bail!("command failed with status {status}");
                }
            } else {
                println!("{cmd}");
            }
        }
    }
    Ok(())
}

pub fn print_init_script(shell: &str) -> Result<()> {
    match shell {
        "zsh" => print!("{}", include_str!("../../../shell/thunder.zsh")),
        "bash" => print!("{}", include_str!("../../../shell/thunder.bash")),
        "fish" => print!("{}", include_str!("../../../shell/thunder.fish")),
        other => bail!("unsupported shell: {other} (supported: zsh, bash, fish)"),
    }
    Ok(())
}

pub fn write_default_config() -> Result<()> {
    let config = ThunderConfig::default();
    thunder_core::save_config(&config)?;
    eprintln!(
        "wrote default config to {}",
        thunder_core::config_path()?.display()
    );
    Ok(())
}

pub fn run_history(config: &ThunderConfig, query: Option<&str>, execute: bool) -> Result<()> {
    let lines = read_history_lines(config.history.max_entries)?;
    let filtered: Vec<String> = lines
        .into_iter()
        .rev()
        .filter(|line| {
            query.is_none_or(|q| line.to_lowercase().contains(&q.to_lowercase()))
        })
        .take(config.history.palette_limit)
        .map(|line| format!("[hist] {line}"))
        .collect();

    if filtered.is_empty() {
        bail!("no history entries found");
    }

    let pick_options = PickOptions {
        height: config.pick.height.clone(),
        reverse: config.pick.reverse,
        prompt: "history> ".into(),
        ..PickOptions::default()
    };

    let selected = pick_lines(&filtered, &pick_options)?;
    let Some(label) = selected.into_iter().next() else {
        return Ok(());
    };

    let cmd = label.trim_start_matches("[hist] ").trim();
    validate_safe_command(cmd)?;
    if execute {
        Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
            .context("failed to run history command")?;
    } else {
        println!("{cmd}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_history() {
        assert!(validate_safe_command("cargo test").is_ok());
        assert!(validate_safe_command("echo\nrm").is_err());
    }
}
