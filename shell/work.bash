# work shell integration for bash: eval "$(work activate bash)"
#
# `work sh-cd` and `work sh-on` print one line, the absolute path of the
# target directory. The wrapper captures it first and acts on it second, so
# a failed resolution returns its status and the directory does not change.
work() {
    case "${1-}" in
        cd) shift; local dir; dir="$(command work sh-cd "$@")" || return $?; cd "$dir" ;;
        on) shift; local dir; dir="$(command work sh-on "$@")" || return $?; cd "$dir" && command work sh-editor ;;
        *)  command work "$@" ;;
    esac
}
