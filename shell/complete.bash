# work tab completion for bash: work completions bash --install
# The binary computes the candidates, so this never goes stale.
_work() {
    local line
    COMPREPLY=()
    while IFS= read -r line; do
        COMPREPLY+=("$line")
    done < <(command work sh-complete bash -- "${COMP_WORDS[@]:1:COMP_CWORD}" 2>/dev/null)
}

if [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -ge 4 || "${BASH_VERSINFO[0]}" -gt 4 ]]; then
    complete -F _work -o nosort work
else
    complete -F _work work
fi
