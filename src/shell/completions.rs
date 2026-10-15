//! Completion scripts and where `--install` puts them.

use std::path::{Path, PathBuf};

use crate::config::Locations;
use crate::env::EnvSnapshot;
use crate::proc;
use crate::shell::Shell;

/// Subcommands whose argument slot completes repo names.
const RESOLVING: [&str; 8] = [
    "cd", "on", "edit", "clone", "browse", "open", "pull", "list",
];

fn fish_hook() -> String {
    format!(
        "
# Repo names from the cache for commands that resolve a repo.
complete -c work -n '__fish_seen_subcommand_from {}' -f -a '(work list --complete 2>/dev/null)'
complete -c work -n '__fish_work_using_subcommand completions' -f -a 'bash zsh fish'
",
        RESOLVING.join(" ")
    )
}

fn bash_hook() -> String {
    format!(
        r#"# Repo names from the cache for commands that resolve a repo.
_work_with_repos() {{
    local cur="${{COMP_WORDS[COMP_CWORD]}}"
    case "${{COMP_WORDS[1]}}" in
        {})
            if [[ "$cur" != -* ]]; then
                local i positional=0
                for ((i = 2; i < COMP_CWORD; i++)); do
                    [[ "${{COMP_WORDS[i]}}" != -* ]] && positional=1
                done
                if [[ $positional -eq 0 ]]; then
                    local IFS=$'\n'
                    COMPREPLY=( $(compgen -W "$(work list --complete 2>/dev/null)" -- "$cur") )
                    return 0
                fi
            fi
            ;;
    esac
    _work "$@"
}}

"#,
        RESOLVING.join("|")
    )
}

fn zsh_hook() -> String {
    format!(
        r#"# Repo names from the cache for commands that resolve a repo.
_work() {{
    case "$words[2]" in
        {})
            if [[ "$words[CURRENT]" != -* ]]; then
                local w positional=0
                for w in "${{(@)words[3,CURRENT-1]}}"; do
                    [[ "$w" != -* ]] && positional=1
                done
                if (( positional == 0 )); then
                    local -a repos
                    repos=(${{(f)"$(work list --complete 2>/dev/null)"}})
                    compadd -a repos
                    return
                fi
            fi
            ;;
    esac
    _work_clap "$@"
}}

"#,
        RESOLVING.join("|")
    )
}

/// A copy of `cmd` without hidden subcommands and hidden arguments, because
/// `clap_complete`'s static generators do not filter `hide = true`.
fn visible(cmd: &clap::Command) -> clap::Command {
    let mut out = clap::Command::new(cmd.get_name().to_owned())
        .args(cmd.get_arguments().filter(|a| !a.is_hide_set()).cloned())
        .subcommands(
            cmd.get_subcommands()
                .filter(|s| !s.is_hide_set())
                .map(visible),
        );
    if let Some(about) = cmd.get_about() {
        out = out.about(about.clone());
    }
    if cmd.is_subcommand_required_set() {
        out = out.subcommand_required(true);
    }
    out
}

/// The completion script for `shell`, completing repo names for commands
/// that resolve a repo.
#[must_use]
pub fn completion_script(shell: Shell, cmd: &mut clap::Command) -> String {
    let mut buf = Vec::new();
    let mut shown = visible(cmd);
    clap_complete::generate(
        clap_complete::Shell::from(shell),
        &mut shown,
        "work",
        &mut buf,
    );
    let base = String::from_utf8_lossy(&buf).into_owned();
    match shell {
        Shell::Fish => format!("{base}{}", fish_hook()),
        Shell::Bash => bash_with_hook(&base),
        Shell::Zsh => zsh_with_hook(&base),
    }
}

fn bash_with_hook(base: &str) -> String {
    const REGISTER: &str = "if [[ \"${BASH_VERSINFO[0]}\" -eq 4";
    let hook = bash_hook();
    let Some(at) = base.rfind(REGISTER) else {
        return format!(
            "{base}\n{hook}complete -F _work_with_repos -o bashdefault -o default work\n"
        );
    };
    let (head, tail) = base.split_at(at);
    format!(
        "{head}{hook}{}",
        tail.replace("complete -F _work ", "complete -F _work_with_repos ")
    )
}

/// The hook takes the name `_work` because compsys keeps calling `_work`
/// after an autoloaded first run, so clap's function becomes `_work_clap`.
fn zsh_with_hook(base: &str) -> String {
    const TRAILER: &str = "if [ \"$funcstack[1]\" = \"_work\" ]; then";
    let renamed = base.replacen("\n_work() {\n", "\n_work_clap() {\n", 1);
    let hook = zsh_hook();
    match renamed.rfind(TRAILER) {
        Some(at) => {
            let (head, tail) = renamed.split_at(at);
            format!("{head}{hook}{tail}")
        }
        None => format!("{renamed}\n{hook}compdef _work work\n"),
    }
}

/// The completion file `--install` writes for `shell`.
#[must_use]
pub fn install_path(
    shell: Shell,
    locations: &Locations,
    homebrew_prefix: Option<&Path>,
) -> PathBuf {
    match (shell, homebrew_prefix) {
        (Shell::Fish, _) => locations.config_home.join("fish/completions/work.fish"),
        (Shell::Bash, Some(prefix)) => prefix.join("etc/bash_completion.d/work"),
        (Shell::Zsh, Some(prefix)) => prefix.join("share/zsh/site-functions/_work"),
        (Shell::Bash, None) => locations.data_home.join("bash-completion/completions/work"),
        (Shell::Zsh, None) => locations.data_home.join("zsh/site-functions/_work"),
    }
}

/// The Homebrew prefix on macOS. Always `None` elsewhere, because distro
/// bash-completion does not read Linuxbrew's directories.
#[must_use]
pub fn homebrew_prefix(env: &EnvSnapshot) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    if let Some(prefix) = &env.homebrew_prefix {
        return Some(prefix.clone());
    }
    let path = env.path.as_ref()?;
    let out = proc::command("brew")
        .arg("--prefix")
        .env("PATH", path)
        .output()
        .ok()
        .filter(|out| out.status.success())?;
    let text = String::from_utf8(out.stdout).ok()?;
    let prefix = PathBuf::from(text.trim());
    prefix.is_absolute().then_some(prefix)
}
