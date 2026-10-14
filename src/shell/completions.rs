//! Completion scripts and where `--install` puts them.

use std::path::{Path, PathBuf};

use crate::config::Locations;
use crate::env::EnvSnapshot;
use crate::shell::Shell;

/// The clap-generated script for `shell` plus the repo-name hook
/// (`work list --complete` in the argument slot of resolving commands).
/// `cmd` is `Cli::command()`.
#[must_use]
pub fn completion_script(shell: Shell, cmd: &mut clap::Command) -> String {
    let _ = (shell, cmd);
    unimplemented!("W2.11")
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
    let _ = (shell, locations, homebrew_prefix);
    unimplemented!("W2.11")
}

/// `$HOMEBREW_PREFIX`, else the output of `brew --prefix` when `brew` is on
/// `env.path`, else `None`.
#[must_use]
pub fn homebrew_prefix(env: &EnvSnapshot) -> Option<PathBuf> {
    let _ = env;
    unimplemented!("W2.11")
}
