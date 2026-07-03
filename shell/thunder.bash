# Thunder shell integration for bash.
# Usage: eval "$(tn i bash)"

if [[ $- == *i* ]]; then
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
    export THUNDER_LAST_CMD="$BASH_COMMAND"
    export THUNDER_STDERR_FILE="$(__thunder_stderr_file)"
    mkdir -p "$(dirname "$THUNDER_STDERR_FILE")"
    : > "$THUNDER_STDERR_FILE"
    exec 3>&2
    exec 2> >(tee "$THUNDER_STDERR_FILE" >&3)
  }

  __thunder_precmd() {
    local exit_code=$?
    export THUNDER_LAST_EXIT="$exit_code"
    if [[ -s "${THUNDER_STDERR_FILE:-}" ]]; then
      export THUNDER_LAST_STDERR="$(<"$THUNDER_STDERR_FILE")"
    else
      export THUNDER_LAST_STDERR=""
    fi

    if [[ -n "${THUNDER_LAST_CMD:-}" ]]; then
      mkdir -p "${XDG_DATA_HOME:-$HOME/.local/share}/thunder"
      if [[ "$THUNDER_LAST_CMD" != tn* && "$THUNDER_LAST_CMD" != thunder* ]]; then
        echo "$THUNDER_LAST_CMD" >> "${XDG_DATA_HOME:-$HOME/.local/share}/thunder/history"
      fi
    fi
  }

  trap '__thunder_preexec' DEBUG
  PROMPT_COMMAND='__thunder_precmd;'"${PROMPT_COMMAND:-:}"

  fix() {
    if [[ "$1" == "-y" || "$1" == "--apply" ]]; then
      tn f -y
    else
      tn f
    fi
  }
fi
