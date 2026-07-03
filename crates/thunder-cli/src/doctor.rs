use std::path::Path;

use anyhow::Result;
use thunder_core::{ThunderConfig, config_path, data_dir, resolve_binary};
use thunder_index::{client_ping, daemon_status};

pub fn run_doctor(config: &ThunderConfig, cwd: &Path) -> Result<()> {
    let root = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());

    println!("Thunder Doctor");
    println!("==============\n");

    check_binary("tn / thunder", std::env::current_exe().ok().as_ref());
    check_binary(
        "ripgrep",
        resolve_binary(&config.search.fallback, &["rg"]).as_ref(),
    );
    check_binary("thunderd", find_thunderd().as_ref());
    check_binary(
        "thefuck (optional)",
        resolve_binary("thefuck", &["/opt/homebrew/bin/thefuck"]).as_ref(),
    );

    println!();
    if config_path().map(|p| p.is_file()).unwrap_or(false) {
        println!("[ok] config: {}", config_path().unwrap().display());
    } else {
        println!("[warn] config missing — run `tn c --init`");
    }

    if data_dir().map(|p| p.is_dir()).unwrap_or(false) {
        println!("[ok] data dir: {}", data_dir().unwrap().display());
    } else {
        println!("[warn] data dir will be created on first use");
    }

    println!();
    match daemon_status(&root) {
        Ok(status) if status.running => {
            println!(
                "[ok] thunderd running ({} lines indexed)",
                status.lines_indexed.unwrap_or(0)
            );
        }
        Ok(_) => println!("[warn] thunderd not running — run `tn d st` for fast search"),
        Err(err) => println!("[warn] daemon check failed: {err}"),
    }

    if client_ping(&root).unwrap_or(false) {
        println!("[ok] daemon socket reachable for {}", root.display());
    }

    println!();
    if std::env::var("THUNDER_LAST_CMD").is_ok() {
        println!("[ok] shell integration: THUNDER_LAST_CMD is set");
    } else {
        println!("[warn] shell integration not loaded — run `eval \"$(tn i zsh)\"`");
    }

    if std::env::var("EDITOR").is_ok() || std::env::var("VISUAL").is_ok() {
        println!("[ok] editor env var set");
    } else if config.general.editor.is_some() {
        println!("[ok] editor configured in config");
    } else {
        println!("[hint] set EDITOR or general.editor for --open support");
    }

    Ok(())
}

fn check_binary(name: &str, path: Option<&std::path::PathBuf>) {
    match path {
        Some(path) if path.exists() || !path.components().any(|c| matches!(c, std::path::Component::Normal(_))) => {
            println!("[ok] {name}: {}", path.display());
        }
        Some(path) => println!("[ok] {name}: {}", path.display()),
        None => println!("[fail] {name}: not found"),
    }
}

fn find_thunderd() -> Option<std::path::PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join("thunderd")))
        .filter(|p| p.exists())
}
