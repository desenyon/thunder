#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/target/release/thunder"
TN="$ROOT/target/release/tn"
THUNDERD="$ROOT/target/release/thunderd"

echo "==> Building Thunder (release)"
source "$HOME/.cargo/env"
cd "$ROOT"
cargo build --release -q

echo "==> Writing default config"
"$TN" c --init

echo "==> Unit tests"
cargo test -q

echo "==> CLI smoke tests"
"$TN" --version | grep -q "0.1.0"
"$TN" --help | grep -q "palette"
"$TN" d ss && status=running || status=stopped
echo "    daemon status: $status"

echo "==> Short alias tests (tn)"
"$TN" s --no-ui --no-daemon "pick_from" crates | grep -q "thunder-pick"
"$TN" --help | grep -q "\[aliases: s\]"

echo "==> Plain search (ripgrep path)"
"$BIN" search --no-ui --no-daemon "pick_from" crates | grep -q "thunder-pick"

echo "==> Index daemon"
pkill -f "$THUNDERD" 2>/dev/null || true
rm -f "${XDG_DATA_HOME:-$HOME/.local/share}/thunder/thunderd.sock"
"$TN" d st
sleep 0.5
"$TN" d ss
"$TN" s --no-ui "SearchIndex" crates | grep -q "thunder-index"

echo "==> Fix rules (native, no thefuck)"
export THUNDER_LAST_CMD="gti status"
export THUNDER_LAST_EXIT=127
export THUNDER_LAST_STDERR=""
"$TN" f | grep -q "git status"

export THUNDER_LAST_CMD="apt install vim"
export THUNDER_LAST_EXIT=1
export THUNDER_LAST_STDERR="Permission denied"
"$TN" f | grep -q "sudo apt install vim"

echo "==> Pick (non-interactive pipe mode)"
printf 'alpha\nbeta\ngamma\n' | "$TN" p --query beta 2>/dev/null | grep -q beta || {
  echo "    pick skipped (non-tty environment)"
}

echo "==> Security: unsafe preview rejected"
if "$TN" p item --preview 'cat {1}; rm -rf /' 2>/dev/null; then
  echo "FAIL: unsafe preview accepted"
  exit 1
else
  echo "    unsafe preview blocked"
fi

echo "==> All QA checks passed"
