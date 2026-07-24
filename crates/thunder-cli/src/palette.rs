use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use thunder_core::{
    ThunderConfig, load_recent_files, open_in_editor, read_history_lines, record_recent_file,
    resolve_preview, validate_preview_command,
};
use thunder_fix::validate_safe_command;
use thunder_index::{client_list_files, client_ping};
use thunder_pick::{PickOptions, pick_lines};
use thunder_search::files::fuzzy_score;

use crate::git::{git_score_bonus, git_status_map};
use crate::scripts::collect_project_scripts;

#[derive(Debug, Clone)]
struct PaletteItem {
    label: String,
    score: f64,
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
    items.extend(collect_recent_items(config, cwd)?);
    items.extend(collect_script_items(cwd)?);
    items.extend(collect_plugin_items(config, cwd)?);
    items.extend(collect_file_items(cwd, config)?);

    if items.is_empty() {
        bail!("palette is empty; run some commands or add files in this directory");
    }

    items.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    items.truncate(config.history.palette_limit);

    let labels: Vec<String> = items.iter().map(|i| i.label.clone()).collect();
    let preview = resolve_preview(config);
    validate_preview_command(&preview)?;

    let mut pick_options = PickOptions::from_config(config);
    pick_options.prompt = "thunder> ".into();
    pick_options.preview_cmd = Some(preview);

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
    for (rank, line) in lines.into_iter().rev().enumerate() {
        let score = 300.0 - rank as f64;
        items.push(PaletteItem {
            label: format!("cmd  {line}"),
            score,
            action: PaletteAction::RerunCommand(line),
        });
    }
    Ok(items)
}

fn collect_recent_items(config: &ThunderConfig, cwd: &Path) -> Result<Vec<PaletteItem>> {
    let recent = load_recent_files(config.history.recent_files_limit)?;
    let mut items = Vec::new();
    for entry in recent {
        let path = PathBuf::from(&entry.path);
        let rel = if path.is_absolute() {
            path.strip_prefix(cwd)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| entry.path.clone())
        } else {
            entry.path.clone()
        };
        items.push(PaletteItem {
            label: format!("rec  {rel}"),
            score: 400.0 + entry.score,
            action: PaletteAction::OpenFile(cwd.join(&rel)),
        });
    }
    Ok(items)
}

fn collect_script_items(cwd: &Path) -> Result<Vec<PaletteItem>> {
    let scripts = collect_project_scripts(cwd)?;
    Ok(scripts
        .into_iter()
        .map(|(label, cmd)| PaletteItem {
            label: format!("run  {label}"),
            score: 350.0,
            action: PaletteAction::RerunCommand(cmd),
        })
        .collect())
}

fn collect_plugin_items(config: &ThunderConfig, cwd: &Path) -> Result<Vec<PaletteItem>> {
    let mut items = Vec::new();
    for plugin in &config.palette.plugin_commands {
        let output = Command::new("sh")
            .arg("-c")
            .arg(plugin)
            .current_dir(cwd)
            .output()
            .with_context(|| format!("palette plugin failed: {plugin}"))?;
        if !output.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let Some((label, score, cmd)) = parse_plugin_line(line) else {
                continue;
            };
            items.push(PaletteItem {
                label: format!("plug {label}"),
                score,
                action: PaletteAction::RerunCommand(cmd),
            });
        }
    }
    Ok(items)
}

fn parse_plugin_line(line: &str) -> Option<(String, f64, String)> {
    let mut parts = line.splitn(3, '|');
    let label = parts.next()?.trim().to_string();
    let score = parts.next()?.trim().parse().ok()?;
    let cmd = parts.next()?.trim().to_string();
    if label.is_empty() || cmd.is_empty() {
        return None;
    }
    Some((label, score, cmd))
}

fn collect_file_items(cwd: &Path, config: &ThunderConfig) -> Result<Vec<PaletteItem>> {
    let root = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let git = git_status_map(cwd);
    let mut items = Vec::new();

    if client_ping(&root).unwrap_or(false) {
        if let Ok(paths) = client_list_files(&root, None, config.history.palette_limit * 2) {
            for rel in paths {
                let score = 100.0 + git_score_bonus(&rel, &git);
                items.push(PaletteItem {
                    label: format!("file {rel}"),
                    score,
                    action: PaletteAction::OpenFile(root.join(&rel)),
                });
            }
            return Ok(items);
        }
    }

    use ignore::WalkBuilder;
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
        let depth_penalty = rel.matches('/').count() as f64 * 2.0;
        let score = 100.0 - depth_penalty + git_score_bonus(&rel, &git);
        items.push(PaletteItem {
            label: format!("file {rel}"),
            score,
            action: PaletteAction::OpenFile(path),
        });
        if items.len() >= config.history.palette_limit {
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
            let _ = record_recent_file(&full, config.history.recent_files_limit);
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
    let filtered: Vec<(f64, String)> = lines
        .into_iter()
        .rev()
        .filter_map(|line| {
            let score = query.map(|q| fuzzy_score(&line, q)).unwrap_or(1.0);
            if query.is_some() && score <= 0.0 {
                return None;
            }
            Some((score, format!("cmd  {line}")))
        })
        .take(config.history.palette_limit)
        .collect();

    if filtered.is_empty() {
        bail!("no history entries found");
    }

    let mut sorted = filtered;
    sorted.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let labels: Vec<String> = sorted.into_iter().map(|(_, l)| l).collect();

    let mut pick_options = PickOptions::from_config(config);
    pick_options.prompt = "history> ".into();

    let selected = pick_lines(&labels, &pick_options)?;
    let Some(label) = selected.into_iter().next() else {
        return Ok(());
    };

    let cmd = label.trim_start_matches("cmd").trim();
    validate_safe_command(cmd)?;
    if execute {
        let status = Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
            .context("failed to run history command")?;
        if !status.success() {
            bail!("history command exited with status {status}");
        }
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
