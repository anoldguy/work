//! Completion scripts and where `--install` puts them.

use std::path::{Path, PathBuf};

use crate::config::Locations;
use crate::env::EnvSnapshot;
use crate::proc;
use crate::shell::Shell;

/// The completion script for `shell`: a stub that asks `work sh-complete`
/// for the candidates on every Tab press, so an installed copy keeps up
/// with the binary.
#[must_use]
pub const fn completion_script(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash => include_str!("../../shell/complete.bash"),
        Shell::Zsh => include_str!("../../shell/complete.zsh"),
        Shell::Fish => include_str!("../../shell/complete.fish"),
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
