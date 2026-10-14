//! Shell detection from `$SHELL`.

use crate::env::EnvSnapshot;
use crate::shell::Shell;

/// The shell named by the basename of `$SHELL`; bash when unset or
/// unrecognised.
#[must_use]
pub fn detect(env: &EnvSnapshot) -> Shell {
    let _ = env;
    unimplemented!("W1.6")
}
