# work tab completion for fish: work completions fish --install
# The binary computes the candidates, so this never goes stale.
function __work_complete
    set -l words (commandline -opc)
    set -e words[1]
    set -l current (commandline -ct)
    command work sh-complete fish -- $words "$current" 2>/dev/null
end

complete -c work -f -k -a '(__work_complete)'
