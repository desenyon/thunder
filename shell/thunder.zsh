# Thunder shell integration for zsh.
# Usage: eval "$(tn i zsh)"

if [[ -o interactive ]]; then
  alias tn='tn'
  alias ts='tn s'
  alias tp='tn p'
  alias tf='tn f'
  alias tfl='tn fl'
  alias th='tn h'
  alias td='tn d'
  alias tpal='tn pal'
  alias tdoc='tn doc'

  __thunder_stderr_file() {
    echo "${XDG_DATA_HOME:-$HOME/.local/share}/thunder/last.stderr"
  }

  __thunder_preexec() {
    export THUNDER_LAST_CMD="$1"
    local stderr_file
    stderr_file="$(__thunder_stderr_file)"
    mkdir -p "${stderr_file:h}"
    : > "$stderr_file"
    exec {THUNDER_STDERR_FD}>&2
    exec 2> >(tee "$stderr_file" >&${THUNDER_STDERR_FD})
  }

  __thunder_precmd() {
    local exit_code=$?
    export THUNDER_LAST_EXIT="$exit_code"
    local stderr_file
    stderr_file="$(__thunder_stderr_file)"
    if [[ -s "$stderr_file" ]]; then
      export THUNDER_LAST_STDERR="$(<"$stderr_file")"
    else
      export THUNDER_LAST_STDERR=""
    fi

    if [[ -n "${THUNDER_LAST_CMD:-}" ]]; then
      if command -v tn >/dev/null 2>&1; then
        :
      fi
      mkdir -p "${XDG_DATA_HOME:-$HOME/.local/share}/thunder"
      if [[ "$THUNDER_LAST_CMD" != "tn"* && "$THUNDER_LAST_CMD" != "thunder"* ]]; then
        echo "$THUNDER_LAST_CMD" >> "${XDG_DATA_HOME:-$HOME/.local/share}/thunder/history"
      fi
    fi
  }

  autoload -Uz add-zsh-hook
  add-zsh-hook preexec __thunder_preexec
  add-zsh-hook precmd __thunder_precmd

  fix() {
    if [[ "$1" == "-y" || "$1" == "--apply" ]]; then
      tn f -y
    else
      tn f
    fi
  }
fi
