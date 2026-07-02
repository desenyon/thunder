use std::io::{self, BufRead, Cursor, IsTerminal, Read};
use std::process::Command;

use anyhow::{Context, Result, bail};
use skim::prelude::*;

/// How to run the interactive picker.
#[derive(Debug, Clone)]
pub struct PickOptions {
    pub multi: bool,
    pub height: String,
    pub reverse: bool,
    pub preview_cmd: Option<String>,
    pub query: Option<String>,
    pub prompt: String,
}

impl Default for PickOptions {
    fn default() -> Self {
        Self {
            multi: false,
            height: "60%".to_string(),
            reverse: true,
            preview_cmd: None,
            query: None,
            prompt: "> ".to_string(),
        }
    }
}

/// Run the picker over newline-delimited items from any reader.
pub fn pick_from_reader<R>(items: R, options: &PickOptions) -> Result<Vec<String>>
where
    R: BufRead + Send + 'static,
{
    let item_reader = SkimItemReader::default();
    let stream = item_reader.of_bufread(items);
    pick_from_stream(stream, options)
}

/// Run the picker over in-memory lines.
pub fn pick_lines(lines: &[String], options: &PickOptions) -> Result<Vec<String>> {
    let input = lines.join("\n");
    pick_from_reader(Cursor::new(input), options)
}

/// Run the picker over stdin when it is piped; otherwise return an error.
pub fn pick_stdin(options: &PickOptions) -> Result<Vec<String>> {
    if io::stdin().is_terminal() {
        bail!("no input: pipe items to thunder pick or pass lines as arguments");
    }

    let mut buffer = String::new();
    io::stdin()
        .read_to_string(&mut buffer)
        .context("failed to read stdin")?;

    if buffer.trim().is_empty() {
        bail!("no input: stdin was empty");
    }

    pick_from_reader(Cursor::new(buffer), options)
}

/// Prefer the embedded skim picker; fall back to the external `fzf` binary when requested.
pub fn pick_with_backend(
    lines: &[String],
    options: &PickOptions,
    use_fzf: bool,
) -> Result<Vec<String>> {
    if use_fzf {
        pick_with_fzf(lines, options)
    } else {
        pick_lines(lines, options)
    }
}

fn pick_from_stream(stream: SkimItemReceiver, options: &PickOptions) -> Result<Vec<String>> {
    let mut builder = SkimOptionsBuilder::default();
    builder
        .height(&options.height)
        .multi(options.multi)
        .reverse(options.reverse)
        .prompt(&options.prompt);

    if let Some(query) = &options.query {
        builder.query(query);
    }

    if let Some(preview) = &options.preview_cmd {
        builder.preview(preview.clone());
    }

    let skim_options = builder.build().context("invalid skim options")?;

    let output = Skim::run_with(skim_options, Some(stream)).map_err(|err| anyhow::anyhow!("{err}"))?;

    Ok(output
        .selected_items
        .iter()
        .map(|item| item.output().to_string())
        .collect())
}

fn pick_with_fzf(lines: &[String], options: &PickOptions) -> Result<Vec<String>> {
    let fzf = which_fzf()?;

    let mut child = Command::new(fzf);
    child.arg("--height").arg(&options.height);

    if options.multi {
        child.arg("--multi");
    }
    if options.reverse {
        child.arg("--reverse");
    }
    if let Some(query) = &options.query {
        child.arg("--query").arg(query);
    }
    if let Some(preview) = &options.preview_cmd {
        child.arg("--preview").arg(preview);
    }

    child
        .arg("--prompt")
        .arg(format!("{} ", options.prompt.trim_end()))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());

    let mut child = child.spawn().context("failed to spawn fzf")?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        for line in lines {
            writeln!(stdin, "{line}").context("failed to write to fzf")?;
        }
    }

    let output = child.wait_with_output().context("fzf exited unexpectedly")?;

    if !output.status.success() && output.status.code() != Some(130) {
        bail!("fzf failed with status {}", output.status);
    }

    let stdout = String::from_utf8(output.stdout).context("fzf output was not valid utf-8")?;
    Ok(stdout
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

fn which_fzf() -> Result<String> {
    for candidate in ["fzf", "/opt/homebrew/bin/fzf", "/usr/local/bin/fzf"] {
        if Command::new(candidate)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
        {
            return Ok(candidate.to_string());
        }
    }

    bail!("fzf not found in PATH; install fzf or omit --fzf")
}

/// Collect lines from args or stdin (when piped).
pub fn collect_input_lines(args: &[String]) -> Result<Vec<String>> {
    if !args.is_empty() {
        return Ok(args.to_vec());
    }

    if io::stdin().is_terminal() {
        bail!("no input: pass items as arguments or pipe to stdin");
    }

    let stdin = io::stdin();
    let mut lines = Vec::new();
    for line in stdin.lock().lines() {
        lines.push(line.context("failed to read stdin")?);
    }

    if lines.is_empty() {
        bail!("no input: stdin was empty");
    }

    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_options_are_valid() {
        let options = PickOptions::default();
        assert_eq!(options.height, "60%");
        assert!(!options.multi);
    }
}
