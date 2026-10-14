//! Completion scripts and where `--install` puts them.

use std::path::{Path, PathBuf};
use std::process::Command;

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

/// The bash wrapper: repo names for the first positional of a resolving
/// command (flags before it are skipped), otherwise clap's `_work`.
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

/// The zsh `_work`: repo names for the first positional of a resolving
/// command, otherwise clap's function (renamed `_work_clap`).
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

/// The clap-generated script for `shell` plus the repo-name hook
/// (`work list --complete` in the argument slot of resolving commands).
/// Hidden subcommands and flags are left out. `cmd` is `Cli::command()`.
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

/// Inserts the wrapper before clap's `complete` registration and points the
/// registration at it.
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

/// Renames clap's `_work` to `_work_clap` and defines the hook as `_work`,
/// leaving clap's trailing block untouched. That works when sourced
/// (`compdef _work work`) and when autoloaded from `fpath`, where compsys
/// keeps calling `_work` after the first run, which is now the hook.
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

/// The completion file `--install` writes for `shell` (PLAN §10 Q4): fish
/// under `locations.config_home`; bash/zsh under `homebrew_prefix` when
/// given, else under `locations.data_home`.
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

/// macOS only: `$HOMEBREW_PREFIX`, else the output of `brew --prefix` when
/// `brew` is on `env.path`, else `None`. Always `None` elsewhere, because
/// distro bash-completion does not read Linuxbrew's directories.
#[must_use]
pub fn homebrew_prefix(env: &EnvSnapshot) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    if let Some(prefix) = &env.homebrew_prefix {
        return Some(prefix.clone());
    }
    let path = env.path.as_ref()?;
    let out = proc::run_capture(Command::new("brew").arg("--prefix").env("PATH", path))
        .ok()
        .filter(|out| out.status.success())?;
    let text = String::from_utf8(out.stdout).ok()?;
    let prefix = PathBuf::from(text.trim());
    prefix.is_absolute().then_some(prefix)
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;
    use crate::cli::Cli;

    fn locations() -> Locations {
        Locations {
            home: PathBuf::from("/h"),
            config_home: PathBuf::from("/h/.config"),
            data_home: PathBuf::from("/h/.local/share"),
            config_file: PathBuf::from("/h/.config/work/config.toml"),
            default_cache_dir: PathBuf::from("/h/.local/state/work"),
            default_work_dir: PathBuf::from("/h/code"),
        }
    }

    fn script(shell: Shell) -> String {
        completion_script(shell, &mut Cli::command())
    }

    #[test]
    fn bash_snapshot() {
        insta::assert_snapshot!(script(Shell::Bash));
    }

    #[test]
    fn zsh_snapshot() {
        insta::assert_snapshot!(script(Shell::Zsh));
    }

    #[test]
    fn fish_snapshot() {
        insta::assert_snapshot!(script(Shell::Fish));
    }

    #[test]
    fn install_paths_without_homebrew() {
        let l = locations();
        assert_eq!(
            install_path(Shell::Fish, &l, None),
            Path::new("/h/.config/fish/completions/work.fish")
        );
        assert_eq!(
            install_path(Shell::Bash, &l, None),
            Path::new("/h/.local/share/bash-completion/completions/work")
        );
        assert_eq!(
            install_path(Shell::Zsh, &l, None),
            Path::new("/h/.local/share/zsh/site-functions/_work")
        );
    }

    #[test]
    fn install_paths_with_homebrew() {
        let l = locations();
        let brew = Path::new("/opt/homebrew");
        assert_eq!(
            install_path(Shell::Bash, &l, Some(brew)),
            Path::new("/opt/homebrew/etc/bash_completion.d/work")
        );
        assert_eq!(
            install_path(Shell::Zsh, &l, Some(brew)),
            Path::new("/opt/homebrew/share/zsh/site-functions/_work")
        );
        assert_eq!(
            install_path(Shell::Fish, &l, Some(brew)),
            Path::new("/h/.config/fish/completions/work.fish")
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn homebrew_prefix_prefers_env_var() {
        let env = EnvSnapshot {
            homebrew_prefix: Some(PathBuf::from("/opt/homebrew")),
            ..EnvSnapshot::default()
        };
        assert_eq!(homebrew_prefix(&env), Some(PathBuf::from("/opt/homebrew")));
    }

    #[test]
    fn homebrew_prefix_none_without_brew() {
        let dir = tempfile::tempdir().unwrap();
        let env = EnvSnapshot {
            path: Some(dir.path().into()),
            ..EnvSnapshot::default()
        };
        assert_eq!(homebrew_prefix(&env), None);
        assert_eq!(homebrew_prefix(&EnvSnapshot::default()), None);
    }

    #[test]
    fn scripts_carry_the_repo_hook() {
        for shell in [Shell::Bash, Shell::Zsh, Shell::Fish] {
            assert!(script(shell).contains("work list --complete"), "{shell}");
        }
    }
}
