# work shell integration for fish: work activate fish | source
# Capture the path first so a failed lookup returns its status without changing directory.
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
