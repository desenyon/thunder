use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ignore::WalkBuilder;
use thunder_core::{ThunderConfig, history_path, validate_preview_command};
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

pub fn run_palette(config: &ThunderConfig, cwd: &Path) -> Result<()> {
    let mut items = Vec::new();
    items.extend(collect_history_items()?);
    items.extend(collect_file_items(cwd)?);

    if items.is_empty() {
        bail!("palette is empty; run some commands or add files in this directory");
    }

    let labels: Vec<String> = items.iter().map(|i| i.label.clone()).collect();
    let pick_options = PickOptions {
        height: config.pick.height.clone(),
        prompt: "thunder> ".into(),
        preview_cmd: Some(config.pick.preview.clone()),
        ..PickOptions::default()
    };
    validate_preview_command(pick_options.preview_cmd.as_ref().unwrap())?;

    let selected = pick_lines(&labels, &pick_options)?;
    let Some(label) = selected.into_iter().next() else {
        return Ok(());
    };

    let item = items
        .into_iter()
        .find(|i| i.label == label)
        .context("selected palette item disappeared")?;

    execute_action(item.action, config, cwd)
}

fn collect_history_items() -> Result<Vec<PaletteItem>> {
    let path = history_path()?;
    let Ok(contents) = fs::read_to_string(path) else {
        return Ok(Vec::new());
    };

    let mut items = Vec::new();
    for line in contents.lines().rev().take(200) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        items.push(PaletteItem {
            label: format!("[hist] {trimmed}"),
            action: PaletteAction::RerunCommand(trimmed.to_string()),
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
        .max_depth(Some(6));
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

fn execute_action(action: PaletteAction, _config: &ThunderConfig, cwd: &Path) -> Result<()> {
    match action {
        PaletteAction::OpenFile(path) => {
            let rel = path.strip_prefix(cwd).unwrap_or(&path);
            println!("{}", rel.display());
        }
        PaletteAction::RerunCommand(cmd) => {
            validate_rerun_command(&cmd)?;
            println!("{cmd}");
        }
    }
    Ok(())
}

fn validate_rerun_command(cmd: &str) -> Result<()> {
    if cmd.trim().is_empty() {
        bail!("empty history command");
    }
    if cmd.contains('\n') {
        bail!("unsafe multiline history command");
    }
    Ok(())
}

pub fn print_init_script(shell: &str) -> Result<()> {
    match shell {
        "zsh" => print!("{}", include_str!("../../../shell/thunder.zsh")),
        "bash" => print!("{}", include_str!("../../../shell/thunder.bash")),
        other => bail!("unsupported shell: {other} (supported: zsh, bash)"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_multiline_rerun() {
        assert!(validate_rerun_command("echo ok").is_ok());
        assert!(validate_rerun_command("echo\nrm").is_err());
    }
}
