<p align="center">
  <img src="https://img.shields.io/github/actions/workflow/status/desenyon/thunder/ci.yml?branch=main&style=for-the-badge" alt="CI" />
  <img src="https://img.shields.io/badge/rust-1.91%2B-orange?style=for-the-badge&logo=rust" alt="Rust" />
  <img src="https://img.shields.io/badge/license-MIT-blue?style=for-the-badge" alt="MIT" />
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Linux-lightgrey?style=for-the-badge" alt="Platform" />
</p>

<h1 align="center">Thunder</h1>

<p align="center">
  <strong>Search. Pick. Fix. Fly.</strong><br/>
  A unified terminal workflow — ripgrep speed, fzf UX, thefuck smarts — in two keystrokes.
</p>

<p align="center">
  <code>tn s auth</code> &nbsp;·&nbsp; <code>tn fl</code> &nbsp;·&nbsp; <code>tn f</code> &nbsp;·&nbsp; <code>tn</code>
</p>

---

## What is Thunder?

Thunder replaces the trio of tools you reach for dozens of times a day:

| You used to run | Now run |
|-----------------|---------|
| `rg foo \| fzf` | `tn s foo` |
| `fd \| fzf` | `tn fl` |
| `thefuck` | `tn f` |
| `fzf` on history | `tn h` |
| Everything at once | `tn` |

**One binary. Two letters. Zero friction.**

## Features

- **Warm index daemon** — sub-millisecond literal search on repeated queries via `thunderd`
- **Streaming ripgrep search** — regex queries stream into the picker as matches arrive
- **Parallel index build** — rayon-powered initial indexing for large repos
- **Memory-mapped line corpus** — line text stored on disk, mmap'd at search time
- **Query router** — bare `tn <query>` routes to search, files, or history automatically
- **Git-aware palette** — modified/staged files ranked higher
- **Project scripts** — `package.json` and Makefile targets in the omni palette
- **Palette plugins** — shell commands in config emit custom palette entries
- **Config hot-reload** — edits to `config.toml` picked up without restart
- **Windows daemon** — TCP transport on Windows (Unix socket on macOS/Linux)
- **Fix rules crate** — `thunder-fix-rules` isolates native correction rules
- **Daemon-backed file finder** — `tn fl` lists files from the warm index when available
- **Ripgrep fallback** — full regex power when the index isn't enough
- **Embedded skim picker** — fzf-quality TUI with a minimal monochrome theme (no clutter)
- **Smart omni palette** — history, recent files, and project files ranked by frecency
- **11 native fix rules** — git, sudo, cd, npm, docker, python, cargo, pip, kubectl, brew, man
- **Per-project daemon** — each repo gets its own socket and index
- **Trigram-accelerated index** — fast candidate filtering on large codebases
- **Fuzzy file matching** — subsequence scoring for `tn fl` and palette
- **Shell integration** — zsh, bash, fish with stderr capture for smart fixes
- **Security hardened** — preview validation, path sandboxing, dangerous command blocking

### v2.0 theme

Thunder 2.0 ships with a clean monochrome TUI. Configure in `~/.config/thunder/config.toml`:

```toml
[theme]
preset = "minimal"   # minimal | bw | dark
minimal_chrome = true  # hide match-count spinner line

[search]
streaming = true       # stream ripgrep results into picker
multi_select = false   # allow selecting multiple search results

[palette]
plugin_commands = []   # e.g. ["./scripts/my-palette.sh"]
```

## Install

### One-line setup (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/desenyon/thunder/main/scripts/install.sh | bash
```

This will:

1. Install Rust (via rustup) and ripgrep if missing
2. Clone/update Thunder to `~/.local/share/thunder`
3. Build and link `tn`, `thunder`, `thunderd` to `~/.local/bin`
4. Write default config (`tn c --init`)
5. Add PATH + shell hooks to your `~/.zshrc`, `~/.bashrc`, or fish config

Then activate in your current shell:

```bash
export PATH="$HOME/.local/bin:$PATH"
eval "$(tn i zsh)"   # or bash / fish
tn doc
```

### Manual install

```bash
git clone https://github.com/desenyon/thunder.git
cd thunder
cargo build --release

export PATH="$PWD/target/release:$PATH"
tn c --init
eval "$(tn i zsh)"    # add to ~/.zshrc
```

### Update

Re-run the one-liner — it pulls latest and rebuilds:

```bash
curl -fsSL https://raw.githubusercontent.com/desenyon/thunder/main/scripts/install.sh | bash
```

### Dependencies

| Tool | Required | Purpose |
|------|----------|---------|
| `rg` (ripgrep) | Yes | Regex search fallback |
| `bat` | No | Nicer previews (auto-detected) |
| `thefuck` | No | Extended fix fallback |
| `fzf` | No | Alternative picker backend |

## Command reference

### Essentials

| Command | What it does |
|---------|--------------|
| `tn` | Omni palette (files + history) |
| `tn QUERY` | Search + pick results |
| `tn s QUERY` | Search codebase |
| `tn fl [QUERY]` | Fuzzy-find files |
| `tn h [QUERY]` | Search command history |
| `tn f` | Suggest fix for last command |
| `tn f -y` | Apply fix immediately |

### Power user

| Command | What it does |
|---------|--------------|
| `tn s QUERY --open` | Search and open in `$EDITOR` |
| `tn s QUERY --json` | JSON output for scripting |
| `tn pal --execute` | Palette with immediate actions |
| `tn d st` | Start index daemon |
| `tn d ss` | Daemon status |
| `tn d sp` | Stop daemon |
| `tn d rs` | Restart daemon |
| `tn d ri` | Force reindex |
| `tn doc` | Health check / diagnostics |
| `tn cmp zsh` | Shell completions |

### Shell aliases

After `eval "$(tn i zsh)"`:

```
ts QUERY   →  tn s QUERY      search
tp         →  tn p            pick
tf         →  tn f            fix
tfl        →  tn fl           files
th         →  tn h            history
td         →  tn d            daemon
tpal       →  tn pal          palette
tdoc       →  tn doc          doctor
fix        →  tn f            (supports -y)
```

## How search routing works

```mermaid
flowchart LR
    Q[tn s query] --> L{literal query?}
    L -->|yes| D{daemon warm?}
    L -->|no regex| R[ripgrep]
    D -->|yes| I[thunderd trigram index]
    D -->|no| R
    I --> P[skim picker]
    R --> P
    P --> O[--open in EDITOR]
```

Literal queries hit the warm per-project index first. Regex or cold starts fall back to ripgrep. Results flow into skim for interactive picking.

## Configuration

Config path: `~/.config/thunder/config.toml`

```toml
[general]
editor = "nvim"          # override $EDITOR
open_on_select = false   # open files after search pick

[search]
use_daemon = true
fallback = "rg"
max_file_size_bytes = 2097152
max_results = 500

[pick]
height = "60%"
preview = "bat -n --color=always {1}"   # auto-detected if omitted
use_fzf = false
reverse = true
prompt = "> "

[fix]
use_thefuck_fallback = true
enabled_rules = ["git", "sudo", "cd", "npm", "docker", "man", "python", "cargo", "pip", "kubectl", "brew"]

[daemon]
auto_start = true
max_results = 500

[history]
max_entries = 2000
palette_limit = 200
```

## Security

- Preview commands validated against shell injection
- Index paths cannot escape project root (`..` blocked)
- Fix corrections blocked for dangerous patterns (`rm -rf /`, etc.)
- Multiline commands rejected on apply
- Daemon sockets created with `0600` permissions
- Explicit `-y` / `--apply` required to run corrections

## Development

```bash
cargo build              # debug build
cargo test               # unit + integration tests
./scripts/qa.sh          # full end-to-end QA
tn doc                   # local health check
```

## Architecture

```
thunder/
├── thunder-cli/     tn + thunder binaries
├── thunder-core/    config, history, security, editor
├── thunder-search/  ripgrep + daemon routing, file search
├── thunder-index/   thunderd daemon, trigram index
├── thunder-pick/    skim / fzf wrapper
└── thunder-fix/     native rules + thefuck fallback
```

## Credits

Built on excellent open source:

- [ripgrep](https://github.com/BurntSushi/ripgrep) — search
- [skim](https://github.com/skim-rs/skim) — fuzzy picker
- [thefuck](https://github.com/nvbn/thefuck) — correction fallback

See [NOTICE](NOTICE) for licenses.

## License

MIT — see [LICENSE](LICENSE).
