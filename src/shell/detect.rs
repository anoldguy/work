//! Shell detection from `$SHELL`.

use std::path::Path;

use crate::env::EnvSnapshot;
use crate::shell::Shell;

/// The shell named by the basename of `$SHELL`; bash when unset or
/// unrecognised.
#[must_use]
pub fn detect(env: &EnvSnapshot) -> Shell {
    let name = env
        .shell
        .as_deref()
        .and_then(|s| Path::new(s).file_name())
        .and_then(|n| n.to_str());
    match name {
        Some("zsh") => Shell::Zsh,
        Some("fish") => Shell::Fish,
        _ => Shell::Bash,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(shell: Option<&str>) -> EnvSnapshot {
        EnvSnapshot {
            shell: shell.map(str::to_owned),
            ..EnvSnapshot::default()
        }
    }

    #[test]
    fn detects_by_basename() {
        assert_eq!(detect(&env(Some("/usr/bin/fish"))), Shell::Fish);
        assert_eq!(detect(&env(Some("/bin/zsh"))), Shell::Zsh);
        assert_eq!(detect(&env(Some("/bin/bash"))), Shell::Bash);
        assert_eq!(detect(&env(Some("zsh"))), Shell::Zsh);
    }

    #[test]
    fn falls_back_to_bash() {
        assert_eq!(detect(&env(None)), Shell::Bash);
        assert_eq!(detect(&env(Some("/bin/tcsh"))), Shell::Bash);
        assert_eq!(detect(&env(Some("/usr/bin/fishy"))), Shell::Bash);
    }
}
