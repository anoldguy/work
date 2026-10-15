# work shell integration for fish: work activate fish | source
#
# `work sh-cd` and `work sh-on` print one line, the absolute path of the
# target directory. The wrapper captures it first and acts on it second, so
# a failed resolution returns its status and the directory does not change.
function work
    switch "$argv[1]"
        case cd
            set -e argv[1]
            set -l dir (command work sh-cd $argv); or return $status
            cd "$dir"
        case on
            set -e argv[1]
            set -l dir (command work sh-on $argv); or return $status
            cd "$dir"; and command work sh-editor
        case '*'
            command work $argv
    end
end
