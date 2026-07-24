mod doctor;
mod git;
mod palette;
mod router;
mod scripts;

use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{Shell, generate};
use thunder_core::{ThunderConfig, load_config, open_in_editor, validate_preview_command};
use thunder_fix::{FixMode, fix_command};
use thunder_index::{client_reindex, daemon_status, ensure_daemon, stop_daemon};
use thunder_pick::{PickOptions, pick_stdin, pick_with_backend};
use thunder_search::files::pick_files;
use thunder_search::{SearchOptions, search_interactive, search_plain};

use crate::doctor::run_doctor;
use crate::palette::{print_init_script, run_history, run_palette, write_default_config};
use crate::router::{QueryRoute, route_query};

const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "\n\nBuilt with ripgrep, skim, and thefuck (see NOTICE)."
);

#[derive(Parser)]
#[command(version = VERSION, propagate_version = true, disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
    #[arg(value_name = "QUERY")]
    query: Vec<String>,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum ShellKind {
    Zsh,
    Bash,
    Fish,
}

impl ShellKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Zsh => "zsh",
            Self::Bash => "bash",
            Self::Fish => "fish",
        }
    }
}

#[derive(Subcommand)]
enum Commands {
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
        #[arg(long)]
        json: bool,
        #[arg(short, long)]
        open: bool,
    },
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
    #[command(visible_aliases = ["f"])]
    Fix {
        #[arg(short = 'y', long)]
        apply: bool,
        #[arg(long, env = "THUNDER_THEFUCK")]
        thefuck: Option<String>,
    },
    #[command(visible_aliases = ["pal", "a"])]
    Palette {
        #[arg(long)]
        execute: bool,
    },
    #[command(visible_aliases = ["fl"])]
    Files {
        query: Option<String>,
        #[arg(long)]
        open: bool,
    },
    #[command(visible_aliases = ["h"])]
    History {
        query: Option<String>,
        #[arg(long)]
        execute: bool,
    },
    #[command(visible_aliases = ["i"])]
    Init {
        #[arg(value_enum)]
        shell: ShellKind,
    },
    #[command(visible_aliases = ["d"])]
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    #[command(visible_aliases = ["c"])]
    Config {
        #[arg(long)]
        init: bool,
    },
    #[command(visible_aliases = ["doc"])]
    Doctor,
    #[command(visible_aliases = ["cmp"])]
    Complete {
        #[arg(value_enum)]
        shell: ShellKind,
    },
}

#[derive(Subcommand)]
enum DaemonAction {
    #[command(visible_alias = "st")]
    Start,
    #[command(visible_alias = "ss")]
    Status,
    #[command(visible_alias = "sp")]
    Stop,
    #[command(visible_alias = "rs")]
    Restart,
    #[command(visible_alias = "ri")]
    Reindex,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = load_config();
    let short = invoked_as_short_binary();

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
            json,
            open,
        }) => {
            run_search_command(
                &config,
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
                json,
                open,
            )?;
        }
        Some(Commands::Pick {
            items,
            fzf,
            multi,
            query,
            preview,
        }) => run_pick_command(&config, items, fzf, multi, query, preview)?,
        Some(Commands::Fix { apply, thefuck }) => {
            let mode = if apply {
                FixMode::Apply
            } else {
                FixMode::Suggest
            };
            let suggestion = fix_command(mode, &config, thefuck.as_deref())?;
            print_fix_output(&suggestion, apply, short)?;
        }
        Some(Commands::Palette { execute }) => {
            let cwd = std::env::current_dir()?;
            run_palette(&config, &cwd, execute)?;
        }
        Some(Commands::Files { query, open }) => {
            let cwd = std::env::current_dir()?;
            let mut pick = PickOptions::from_config(&config);
            pick.prompt = "files> ".into();
            pick.query = query.clone();
            let selected = pick_files(&cwd, query.as_deref(), &pick, 1000)?;
            for file in selected {
                if open || config.general.open_on_select {
                    open_in_editor(&config, &file.path, None)?;
                } else {
                    println!("{}", file.display);
                }
            }
        }
        Some(Commands::History { query, execute }) => {
            run_history(&config, query.as_deref(), execute)?;
        }
        Some(Commands::Init { shell }) => print_init_script(shell.as_str())?,
        Some(Commands::Daemon { action }) => run_daemon_action(action, &config)?,
        Some(Commands::Config { init }) => {
            if init {
                write_default_config()?;
            } else {
                let cmd = if short { "tn c --init" } else { "thunder config --init" };
                eprintln!("usage: {cmd}");
            }
        }
        Some(Commands::Doctor) => {
            let cwd = std::env::current_dir()?;
            run_doctor(&config, &cwd)?;
        }
        Some(Commands::Complete { shell }) => {
            print_completions(shell);
        }
        None if !cli.query.is_empty() => {
            let query = cli.query.join(" ");
            let cwd = std::env::current_dir()?;
            match route_query(&query, &cwd, &config) {
                QueryRoute::Files => {
                    let mut pick = thunder_pick::PickOptions::from_config(&config);
                    pick.prompt = "files> ".into();
                    pick.query = Some(query.clone());
                    let selected = pick_files(&cwd, Some(&query), &pick, 1000)?;
                    for file in selected {
                        if config.general.open_on_select {
                            open_in_editor(&config, &file.path, None)?;
                        } else {
                            println!("{}", file.display);
                        }
                    }
                }
                QueryRoute::History => {
                    run_history(&config, Some(&query), false)?;
                }
                QueryRoute::Search => {
                    run_search_command(
                        &config,
                        query,
                        vec![],
                        false,
                        false,
                        false,
                        false,
                        config.search.multi_select,
                        None,
                        None,
                        false,
                        false,
                        false,
                    )?;
                }
            }
        }
        None => {
            if io::stdin().is_terminal() {
                let cwd = std::env::current_dir()?;
                run_palette(&config, &cwd, false)?;
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

#[allow(clippy::too_many_arguments)]
fn run_search_command(
    config: &ThunderConfig,
    query: String,
    paths: Vec<PathBuf>,
    ignore_case: bool,
    fixed_strings: bool,
    no_ui: bool,
    fzf: bool,
    multi: bool,
    preview: Option<String>,
    rg: Option<String>,
    no_daemon: bool,
    json: bool,
    open: bool,
) -> Result<()> {
    let mut options = SearchOptions::from_config(config.clone());
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
    options.json_output = json;

    // --json is for scripting: emit a single JSON document and skip the picker.
    if no_ui || json {
        search_plain(&query, &options)?;
        return Ok(());
    }

    let selected = search_interactive(&query, &options)?;
    let cwd = options.cwd.clone();
    for m in selected {
        if open || config.general.open_on_select {
            let path = if m.path.is_absolute() {
                m.path.clone()
            } else {
                cwd.join(&m.path)
            };
            open_in_editor(config, &path, Some(m.line_number))?;
        } else {
            println!("{}", m.path_line());
        }
    }
    Ok(())
}

fn run_pick_command(
    config: &ThunderConfig,
    items: Vec<String>,
    fzf: bool,
    multi: bool,
    query: Option<String>,
    preview: Option<String>,
) -> Result<()> {
    if let Some(preview) = &preview {
        validate_preview_command(preview)?;
    }
    let mut options = PickOptions::from_config(config);
    options.multi = multi;
    options.preview_cmd = preview.or_else(|| options_preview(config));
    options.query = query;

    let selected = if items.is_empty() {
        pick_stdin(&options)?
    } else {
        pick_with_backend(&items, &options, fzf || config.pick.use_fzf)?
    };

    for line in selected {
        println!("{line}");
    }
    Ok(())
}

fn options_preview(config: &ThunderConfig) -> Option<String> {
    Some(thunder_core::resolve_preview(config))
}

fn run_daemon_action(action: DaemonAction, config: &ThunderConfig) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = cwd.canonicalize().unwrap_or(cwd);

    match action {
        DaemonAction::Start => {
            ensure_daemon(root.clone(), config)?;
            eprintln!("thunderd is running for {}", root.display());
        }
        DaemonAction::Status => {
            let status = daemon_status(&root)?;
            if status.running {
                eprintln!(
                    "thunderd: running ({} lines) — {}",
                    status.lines_indexed.unwrap_or(0),
                    root.display()
                );
            } else {
                eprintln!("thunderd: not running for {}", root.display());
                std::process::exit(1);
            }
        }
        DaemonAction::Stop => {
            if stop_daemon(&root)? {
                eprintln!("thunderd: stopped");
            } else {
                eprintln!("thunderd: not running");
            }
        }
        DaemonAction::Restart => {
            stop_daemon(&root)?;
            ensure_daemon(root.clone(), config)?;
            eprintln!("thunderd: restarted for {}", root.display());
        }
        DaemonAction::Reindex => {
            let count = client_reindex(&root)?;
            eprintln!("thunderd: reindexed {count} lines");
        }
    }
    Ok(())
}

fn print_completions(shell: ShellKind) {
    let mut cmd = Cli::command();
    let shell = match shell {
        ShellKind::Zsh => Shell::Zsh,
        ShellKind::Bash => Shell::Bash,
        ShellKind::Fish => Shell::Fish,
    };
    generate(shell, &mut cmd, "tn", &mut io::stdout());
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
            .stdout(predicate::str::contains("search"));
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
