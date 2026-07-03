# Thunder shell integration for fish.
# Usage: tn i fish | source

if status is-interactive
    alias tn tn
    alias ts 'tn s'
    alias tp 'tn p'
    alias tf 'tn f'
    alias tfl 'tn fl'
    alias th 'tn h'
    alias td 'tn d'
    alias tpal 'tn pal'
    alias tdoc 'tn doc'

    function __thunder_stderr_file
        if set -q XDG_DATA_HOME
            echo "$XDG_DATA_HOME/thunder/last.stderr"
        else
            echo "$HOME/.local/share/thunder/last.stderr"
        end
    end

    function __thunder_preexec --on-event fish_preexec
        set -gx THUNDER_LAST_CMD $argv[1]
        set -l stderr_file (__thunder_stderr_file)
        mkdir -p (dirname $stderr_file)
        echo -n > $stderr_file
    end

    function __thunder_postexec --on-event fish_postexec
        set -gx THUNDER_LAST_EXIT $status
        set -l stderr_file (__thunder_stderr_file)
        if test -s $stderr_file
            set -gx THUNDER_LAST_STDERR (cat $stderr_file)
        else
            set -gx THUNDER_LAST_STDERR ""
        end

        if test -n "$THUNDER_LAST_CMD"
            if not string match -q 'tn*' -- $THUNDER_LAST_CMD
                and not string match -q 'thunder*' -- $THUNDER_LAST_CMD
                mkdir -p (dirname $stderr_file)
                echo $THUNDER_LAST_CMD >> (dirname $stderr_file)/history
            end
        end
    end

    function fix
        if test "$argv[1]" = -y -o "$argv[1]" = --apply
            tn f -y
        else
            tn f
        end
    end
end
