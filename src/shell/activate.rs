//! The embedded wrapper scripts printed by `work activate`.

use crate::shell::Shell;

/// The wrapper for `shell`, byte-for-byte from `shell/work.<shell>`
/// (embedded with `include_str!`).
#[must_use]
pub fn activation_script(shell: Shell) -> &'static str {
    let _ = shell;
    unimplemented!("W1.6")
}
