use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;

use anyhow::{Context, Result, bail};
use thunder_pick::{pick_from_channel, pick_with_backend};

use crate::{SearchMatch, SearchOptions, parse_match_line, resolve_rg};

pub fn search_interactive_streaming(query: &str, options: &SearchOptions) -> Result<Vec<SearchMatch>> {
    if options.json_output {
        return super::search_interactive(query, options);
    }

    let (tx, rx) = mpsc::channel::<String>();
    let (match_tx, match_rx) = mpsc::channel::<SearchMatch>();
    let query_owned = query.to_string();
    let options_owned = options.clone();

    let producer = thread::spawn(move || {
        if let Err(err) = stream_ripgrep_matches(&query_owned, &options_owned, tx, match_tx) {
            eprintln!("streaming search error: {err}");
        }
    });

    let mut pick_options = options.pick.clone();
    if let Some(preview) = &options.preview_cmd {
        thunder_core::validate_preview_command(preview)?;
        pick_options.preview_cmd = Some(preview.clone());
    }

    let selected = if options.use_fzf {
        // fzf needs full input — collect channel first
        let mut lines = Vec::new();
        while let Ok(line) = rx.recv() {
            lines.push(line);
        }
        let _ = producer.join();
        pick_with_backend(&lines, &pick_options, true)?
    } else {
        let selected = pick_from_channel(rx, &pick_options)?;
        let _ = producer.join();
        selected
    };

    let mut by_display = HashMap::new();
    while let Ok(m) = match_rx.try_recv() {
        by_display.insert(m.display_line(), m);
    }

    Ok(selected
        .into_iter()
        .filter_map(|line| by_display.get(&line).cloned())
        .collect())
}

fn stream_ripgrep_matches(
    query: &str,
    options: &SearchOptions,
    display_tx: mpsc::Sender<String>,
    match_tx: mpsc::Sender<SearchMatch>,
) -> Result<()> {
    let rg = resolve_rg(options)?;
    let mut command = Command::new(&rg);
    command
        .arg("--json")
        .arg("--line-number")
        .arg("--column")
        .arg("--no-heading")
        .arg("--color=never")
        .arg("--max-count")
        .arg(options.config.search.max_results.to_string())
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

    let stdout = child.stdout.take().context("ripgrep stdout missing")?;
    let reader = BufReader::new(stdout);

    for line in reader.lines() {
        let line = line.context("read ripgrep line")?;
        if let Some(search_match) = parse_match_line(&line)? {
            let display = search_match.display_line();
            if display_tx.send(display).is_err() {
                break;
            }
            if match_tx.send(search_match).is_err() {
                break;
            }
        }
    }

    let status = child.wait().context("ripgrep wait")?;
    if !status.success() && status.code() != Some(1) {
        bail!("ripgrep failed with status {status}");
    }
    Ok(())
}
