#compdef work
# work tab completion for zsh: work completions zsh --install
# The binary computes the candidates, so this never goes stale.
_work() {
    local -a candidates
    candidates=("${(@f)$(command work sh-complete zsh -- "${(@)words[2,CURRENT]}" 2>/dev/null)}")
    [[ -n "${candidates[1]}" ]] || return 1
    _describe -V -t work 'work' candidates -M 'm:{a-zA-Z}={A-Za-z}'
}

if [ "$funcstack[1]" = "_work" ]; then
    _work "$@"
else
    compdef _work work
fi
