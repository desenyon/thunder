# Thunder shell integration for bash.
# Usage: eval "$(tn i bash)"  or  eval "$(thunder init bash)"

if [[ $- == *i* ]]; then
  alias tn='tn'
  alias ts='tn s'
  alias tp='tn p'
  alias tf='tn f'
  alias td='tn d'
  alias tpal='tn pal'

  alias th='tn'
  alias ths='tn s'
  alias thp='tn p'
  alias thf='tn f'

  __thunder_preexec() {
    export THUNDER_LAST_CMD="$BASH_COMMAND"
  }

  __thunder_precmd() {
    local exit_code=$?
    export THUNDER_LAST_EXIT="$exit_code"
    export THUNDER_LAST_STDERR=""

    if [[ -n "${THUNDER_LAST_CMD:-}" ]]; then
      mkdir -p "${XDG_DATA_HOME:-$HOME/.local/share}/thunder"
      echo "$THUNDER_LAST_CMD" >> "${XDG_DATA_HOME:-$HOME/.local/share}/thunder/history"
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
