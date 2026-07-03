#!/usr/bin/env bash
# Thunder one-shot installer
# Usage: curl -fsSL https://raw.githubusercontent.com/desenyon/thunder/main/scripts/install.sh | bash

set -euo pipefail

THUNDER_REPO="${THUNDER_REPO:-https://github.com/desenyon/thunder.git}"
THUNDER_INSTALL_DIR="${THUNDER_INSTALL_DIR:-$HOME/.local/share/thunder}"
THUNDER_BIN_DIR="${THUNDER_BIN_DIR:-$HOME/.local/bin}"
THUNDER_BRANCH="${THUNDER_BRANCH:-main}"

info() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m==>\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

detect_shell() {
  case "${SHELL:-}" in
    */zsh)  echo "zsh" ;;
    */fish) echo "fish" ;;
    *)      echo "bash" ;;
  esac
}

ensure_path_line() {
  local rc_file="$1"
  local path_line='export PATH="$HOME/.local/bin:$PATH"'

  touch "$rc_file"
  if ! grep -qF '.local/bin' "$rc_file" 2>/dev/null; then
    {
      echo ''
      echo '# Thunder / local binaries'
      echo "$path_line"
    } >> "$rc_file"
    info "added ~/.local/bin to PATH in $rc_file"
  fi
}

ensure_shell_hook() {
  local shell_name="$1"
  local rc_file hook_line

  case "$shell_name" in
    zsh)
      rc_file="$HOME/.zshrc"
      hook_line='eval "$(tn i zsh 2>/dev/null)"'
      ;;
    bash)
      rc_file="$HOME/.bashrc"
      hook_line='eval "$(tn i bash 2>/dev/null)"'
      ;;
    fish)
      rc_file="$HOME/.config/fish/config.fish"
      hook_line='tn i fish 2>/dev/null | source'
      mkdir -p "$(dirname "$rc_file")"
      ;;
    *)
      warn "unknown shell; add integration manually with: eval \"\$(tn i zsh)\""
      return 0
      ;;
  esac

  ensure_path_line "$rc_file"

  if ! grep -qF 'tn i' "$rc_file" 2>/dev/null; then
    {
      echo ''
      echo '# Thunder shell integration'
      echo "$hook_line"
    } >> "$rc_file"
    info "added Thunder hooks to $rc_file"
  else
    info "Thunder hooks already present in $rc_file"
  fi
}

ensure_rust() {
  if command -v cargo >/dev/null 2>&1; then
    return 0
  fi

  if ! command -v curl >/dev/null 2>&1; then
    die "cargo not found and curl unavailable — install Rust from https://rustup.rs"
  fi

  info "installing Rust via rustup"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  # shellcheck source=/dev/null
  source "$HOME/.cargo/env"
  command -v cargo >/dev/null 2>&1 || die "rustup install failed"
}

ensure_ripgrep() {
  if command -v rg >/dev/null 2>&1; then
    return 0
  fi

  info "ripgrep not found — attempting install"
  if command -v brew >/dev/null 2>&1; then
    brew install ripgrep
  elif command -v apt-get >/dev/null 2>&1; then
    sudo apt-get update -qq && sudo apt-get install -y ripgrep
  elif command -v pacman >/dev/null 2>&1; then
    sudo pacman -S --noconfirm ripgrep
  else
    warn "install ripgrep manually (required for tn s): https://github.com/BurntSushi/ripgrep"
  fi
}

clone_or_update() {
  if [[ -d "$THUNDER_INSTALL_DIR/.git" ]]; then
    info "updating existing install at $THUNDER_INSTALL_DIR"
    git -C "$THUNDER_INSTALL_DIR" fetch origin "$THUNDER_BRANCH"
    git -C "$THUNDER_INSTALL_DIR" checkout "$THUNDER_BRANCH"
    git -C "$THUNDER_INSTALL_DIR" pull --ff-only origin "$THUNDER_BRANCH"
  else
    info "cloning Thunder to $THUNDER_INSTALL_DIR"
    mkdir -p "$(dirname "$THUNDER_INSTALL_DIR")"
    git clone --depth 1 --branch "$THUNDER_BRANCH" "$THUNDER_REPO" "$THUNDER_INSTALL_DIR"
  fi
}

build_and_link() {
  info "building release binaries"
  # shellcheck source=/dev/null
  [[ -f "$HOME/.cargo/env" ]] && source "$HOME/.cargo/env"
  cargo build --release --manifest-path "$THUNDER_INSTALL_DIR/Cargo.toml" -q

  mkdir -p "$THUNDER_BIN_DIR"
  for bin in tn thunder thunderd; do
    ln -sf "$THUNDER_INSTALL_DIR/target/release/$bin" "$THUNDER_BIN_DIR/$bin"
  done
  info "linked tn, thunder, thunderd → $THUNDER_BIN_DIR"
}

configure() {
  export PATH="$THUNDER_BIN_DIR:$PATH"
  info "writing default config"
  tn c --init
}

main() {
  info "Thunder installer"
  ensure_rust
  ensure_ripgrep
  clone_or_update
  build_and_link
  configure

  local shell_name
  shell_name="$(detect_shell)"
  ensure_shell_hook "$shell_name"

  cat <<EOF

Thunder installed successfully.

  Binaries:  $THUNDER_BIN_DIR/{tn,thunder,thunderd}
  Source:    $THUNDER_INSTALL_DIR
  Config:    ~/.config/thunder/config.toml

Activate in this shell now:

  export PATH="\$HOME/.local/bin:\$PATH"
  eval "\$(tn i $shell_name)"

Then try:

  tn doc        # health check
  tn d st       # start search index
  tn s main     # search your project
  tn            # omni palette

EOF
}

main "$@"
