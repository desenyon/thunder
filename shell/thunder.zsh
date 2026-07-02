# Thunder shell integration for zsh.
# Usage: eval "$(tn i zsh)"  or  eval "$(thunder init zsh)"

if [[ -o interactive ]]; then
  # Primary short command
  alias tn='tn'
  alias ts='tn s'
  alias tp='tn p'
  alias tf='tn f'
  alias td='tn d'
  alias tpal='tn pal'

  # Legacy aliases
  alias th='tn'
  alias ths='tn s'
  alias thp='tn p'
  alias thf='tn f'

  __thunder_preexec() {
    export THUNDER_LAST_CMD="$1"
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
