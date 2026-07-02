use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use thunder_core::load_config;
use thunder_index::run_daemon;

#[derive(Parser)]
#[command(name = "thunderd", about = "Thunder search index daemon")]
struct Args {
    #[arg(long, default_value = ".")]
    root: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let root = args.root.canonicalize().unwrap_or(args.root);
    let config = load_config();
    run_daemon(root, config)
}
