//! The environment each git child runs with.

use std::ffi::OsString;
use std::fmt;
use std::process::Command;

use crate::auth::GitAuth;
use crate::env::GitConfigCount;

/// Environment changes for one git child.
///
/// Always sets `GIT_TERMINAL_PROMPT=0` and `GIT_ALLOW_PROTOCOL=https:ssh:file`.
/// With auth it also appends `GIT_CONFIG_KEY_n`/`VALUE_n` after the user's
/// existing entries (raising `GIT_CONFIG_COUNT`) and removes the trace
/// variables. `Debug` prints variable names only, never values.
pub struct GitEnv {
    /// Variables to set.
    pub set: Vec<(OsString, OsString)>,
    /// Variables to remove.
    pub remove: Vec<OsString>,
}

impl GitEnv {
    /// Applies the changes to `cmd`.
    pub fn apply(&self, cmd: &mut Command) {
        for name in &self.remove {
            cmd.env_remove(name);
        }
        cmd.envs(self.set.iter().map(|(k, v)| (k, v)));
    }
}

impl fmt::Debug for GitEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitEnv")
            .field("set", &self.set.iter().map(|(k, _)| k).collect::<Vec<_>>())
            .field("remove", &self.remove)
            .finish()
    }
}

/// Builds the environment for one git child. `config_count` is the user's
/// `GIT_CONFIG_COUNT` (with [`GitConfigCount::Invalid`] no auth entries are
/// added); `trace_vars` are the inherited trace variable names.
#[must_use]
pub fn git_env(
    auth: Option<&GitAuth<'_>>,
    config_count: GitConfigCount,
    trace_vars: &[OsString],
) -> GitEnv {
    let _ = (auth, config_count, trace_vars);
    unimplemented!("W2.7")
}
