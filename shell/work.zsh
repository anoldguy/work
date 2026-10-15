# work shell integration for zsh: eval "$(work activate zsh)"
# Capture the path first so a failed lookup returns its status without changing directory.
work() {
    case "${1-}" in
        cd) shift; local dir; dir="$(command work sh-cd "$@")" || return $?; cd "$dir" ;;
        on) shift; local dir; dir="$(command work sh-on "$@")" || return $?; cd "$dir" && command work sh-editor ;;
        *)  command work "$@" ;;
    esac
}
