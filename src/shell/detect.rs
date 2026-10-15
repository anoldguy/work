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
