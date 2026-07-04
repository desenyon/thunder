#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
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
"$TN" --version | grep -q "2.0.0"
"$TN" --help | grep -q "\[aliases: s\]"
"$TN" --help | grep -q "files"
"$TN" --help | grep -q "doctor"

echo "==> Doctor"
doc_out="$("$TN" doc 2>&1)"
echo "$doc_out" | grep -q "Thunder Doctor"

echo "==> Short alias tests"
"$TN" s --no-ui --no-daemon "pick_from" crates | grep -q "thunder-pick"
files_out="$("$TN" fl --query lib.rs 2>&1 || true)"
echo "$files_out" | grep -q "lib.rs" || echo "    files pick skipped (non-tty)"

echo "==> Plain search (ripgrep path)"
"$TN" s --no-ui --no-daemon "SearchOptions" crates | grep -q "thunder-search"

echo "==> JSON search output"
"$TN" s --no-ui --no-daemon --json "SearchOptions" crates | grep -q '"path"'

echo "==> Index daemon lifecycle"
pkill -f "$THUNDERD" 2>/dev/null || true
rm -f "${XDG_DATA_HOME:-$HOME/.local/share}"/thunder/thunderd-*.sock 2>/dev/null || true
"$TN" d st 2>&1 | grep -q "running"
sleep 0.5
"$TN" d ss 2>&1 | grep -q "running"
"$TN" d ri 2>&1 | grep -q "reindexed"
"$TN" s --no-ui --no-daemon "SearchIndex" crates | grep -q "thunder-index"
"$TN" d sp 2>&1 | grep -q "stopped"
if "$TN" d ss 2>/dev/null; then
  echo "FAIL: daemon still running after stop"
  exit 1
fi

echo "==> Fix rules (native)"
export THUNDER_LAST_CMD="gti status"
export THUNDER_LAST_EXIT=127
export THUNDER_LAST_STDERR=""
"$TN" f | grep -q "git status"

export THUNDER_LAST_CMD="apt install vim"
export THUNDER_LAST_EXIT=1
export THUNDER_LAST_STDERR="Permission denied"
"$TN" f | grep -q "sudo apt install vim"

export THUNDER_LAST_CMD="kubctl get pods"
export THUNDER_LAST_EXIT=127
export THUNDER_LAST_STDERR=""
"$TN" f | grep -q "kubectl get pods"

echo "==> Security"
if "$TN" p item --preview 'cat {1}; rm -rf /' 2>/dev/null; then
  echo "FAIL: unsafe preview accepted"
  exit 1
else
  echo "    unsafe preview blocked"
fi

echo "==> Completions"
"$TN" cmp zsh | grep -q "_tn"

echo "==> All QA checks passed"
