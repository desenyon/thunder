mod palette;

use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use thunder_core::{load_config, validate_preview_command};
use thunder_fix::{FixMode, fix_command};
use thunder_index::{client_ping, ensure_daemon};
use thunder_pick::{PickOptions, pick_stdin, pick_with_backend};
use thunder_search::{SearchOptions, search_interactive, search_plain};

use crate::palette::{print_init_script, run_palette, write_default_config};

const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "\n\nBuilt with ripgrep, skim, and thefuck (see NOTICE)."
);

/// Fast search, fuzzy pick, and command fix — unified terminal tool.
///
/// Short alias binary: `tn` (same as `thunder`).
/// Short subcommands: `s` `p` `f` `d` `i` `c` and more — see `tn --help`.
#[derive(Parser)]
#[command(
    version = VERSION,
    propagate_version = true,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Shortcut: `tn QUERY` runs search interactively.
    #[arg(value_name = "QUERY")]
    query: Vec<String>,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum ShellKind {
    Zsh,
    Bash,
}

impl ShellKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Zsh => "zsh",
            Self::Bash => "bash",
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Search file contents (index daemon or ripgrep) and pick from results.
    #[command(visible_aliases = ["s"])]
    Search {
        query: String,
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
        #[arg(short, long)]
        ignore_case: bool,
        #[arg(short = 'F', long)]
        fixed_strings: bool,
        #[arg(long)]
        no_ui: bool,
        #[arg(long)]
        fzf: bool,
        #[arg(short, long)]
        multi: bool,
        #[arg(long)]
        preview: Option<String>,
        #[arg(long, env = "THUNDER_RG")]
        rg: Option<String>,
        #[arg(long)]
        no_daemon: bool,
    },

    /// Fuzzy-find and select from stdin or arguments.
    #[command(visible_aliases = ["p"])]
    Pick {
        items: Vec<String>,
        #[arg(long)]
        fzf: bool,
        #[arg(short, long)]
        multi: bool,
        #[arg(short, long)]
        query: Option<String>,
        #[arg(long)]
        preview: Option<String>,
    },

    /// Suggest or apply a fix for the previous shell command.
    #[command(visible_aliases = ["f"])]
    Fix {
        #[arg(short = 'y', long)]
        apply: bool,
        #[arg(long, env = "THUNDER_THEFUCK")]
        thefuck: Option<String>,
    },

    /// Omni palette: history + files + search launcher.
    #[command(visible_aliases = ["pal", "a"])]
    Palette,

    /// Shell integration and config helpers.
    #[command(visible_aliases = ["i"])]
    Init {
        #[arg(value_enum)]
        shell: ShellKind,
    },

    /// Manage the search index daemon.
    #[command(visible_aliases = ["d"])]
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },

    /// Write the default config file.
    #[command(visible_aliases = ["c"])]
    Config {
        #[arg(long)]
        init: bool,
    },
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Start thunderd for the current directory tree.
    #[command(visible_alias = "st")]
    Start,
    /// Show daemon status.
    #[command(visible_alias = "ss")]
    Status,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = load_config();
    let invoked_as_tn = invoked_as_short_binary();

    match cli.command {
        Some(Commands::Search {
            query,
            paths,
            ignore_case,
            fixed_strings,
            no_ui,
            fzf,
            multi,
            preview,
            rg,
            no_daemon,
        }) => {
            let mut options = SearchOptions::from_config(config);
            options.paths = paths;
            options.case_insensitive = ignore_case;
            options.fixed_strings = fixed_strings;
            options.rg_path = rg;
            options.use_fzf = fzf || options.use_fzf;
            if let Some(preview) = preview {
                validate_preview_command(&preview)?;
                options.preview_cmd = Some(preview);
            }
            options.pick.multi = multi;
            options.use_daemon = !no_daemon;

            if no_ui {
                search_plain(&query, &options)?;
            } else {
                let selected = search_interactive(&query, &options)?;
                for m in selected {
                    println!("{}", m.path_line());
                }
            }
        }
        Some(Commands::Pick {
            items,
            fzf,
            multi,
            query,
            preview,
        }) => {
            if let Some(preview) = &preview {
                validate_preview_command(preview)?;
            }
            let options = PickOptions {
                multi,
                preview_cmd: preview.or(Some(config.pick.preview.clone())),
                query,
                height: config.pick.height.clone(),
                ..PickOptions::default()
            };

            let selected = if items.is_empty() {
                pick_stdin(&options)?
            } else {
                pick_with_backend(&items, &options, fzf || config.pick.use_fzf)?
            };

            for line in selected {
                println!("{line}");
            }
        }
        Some(Commands::Fix { apply, thefuck }) => {
            let mode = if apply {
                FixMode::Apply
            } else {
                FixMode::Suggest
            };
            let suggestion = fix_command(mode, &config, thefuck.as_deref())?;
            print_fix_output(&suggestion, apply, invoked_as_tn)?;
        }
        Some(Commands::Palette) => {
            let cwd = std::env::current_dir()?;
            run_palette(&config, &cwd)?;
        }
        Some(Commands::Init { shell }) => {
            print_init_script(shell.as_str())?;
        }
        Some(Commands::Daemon { action }) => {
            match action {
                DaemonAction::Start => {
                    let cwd = std::env::current_dir()?;
                    ensure_daemon(cwd, &config)?;
                    eprintln!("thunderd is running");
                }
                DaemonAction::Status => {
                    if client_ping()? {
                        eprintln!("thunderd: running");
                    } else {
                        eprintln!("thunderd: not running");
                        std::process::exit(1);
                    }
                }
            }
        }
        Some(Commands::Config { init }) => {
            if init {
                write_default_config()?;
            } else {
                let cmd = if invoked_as_tn { "tn c --init" } else { "thunder config --init" };
                eprintln!("usage: {cmd}");
            }
        }
        None if !cli.query.is_empty() => {
            let query = cli.query.join(" ");
            let options = SearchOptions::from_config(config);
            let selected = search_interactive(&query, &options)?;
            for m in selected {
                println!("{}", m.path_line());
            }
        }
        None => {
            if io::stdin().is_terminal() {
                let cwd = std::env::current_dir()?;
                run_palette(&load_config(), &cwd)?;
            } else {
                let options = PickOptions::default();
                let selected = pick_stdin(&options)?;
                for line in selected {
                    println!("{line}");
                }
            }
        }
    }

    Ok(())
}

fn invoked_as_short_binary() -> bool {
    std::env::args_os()
        .next()
        .and_then(|arg| arg.into_string().ok())
        .map(|name| {
            let base = name.rsplit('/').next().unwrap_or(&name);
            base == "tn" || base == "tn.exe"
        })
        .unwrap_or(false)
}

fn print_fix_output(suggestion: &str, apply: bool, short: bool) -> Result<()> {
    if apply {
        println!("{suggestion}");
        return Ok(());
    }

    let apply_hint = if short {
        "Run `tn f -y` to use this correction."
    } else {
        "Run `thunder fix --apply` to use this correction."
    };

    let mut stdout = io::stdout().lock();
    writeln!(stdout, "Suggested fix:")?;
    writeln!(stdout, "  {suggestion}")?;
    writeln!(stdout)?;
    writeln!(stdout, "{apply_hint}")?;
    Ok(())
}

#[cfg(test)]
mod cli_tests {
    use assert_cmd::Command;
    use predicates::prelude::*;

    #[test]
    fn help_works() {
        Command::cargo_bin("thunder")
            .unwrap()
            .arg("--help")
            .assert()
            .success()
            .stdout(predicate::str::contains("Fast search"));
    }

    #[test]
    fn version_lists_attribution() {
        Command::cargo_bin("thunder")
            .unwrap()
            .arg("--version")
            .assert()
            .success()
            .stdout(predicate::str::contains("ripgrep"));
    }
}
