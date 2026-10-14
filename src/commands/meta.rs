//! `version`, `commands --sh`, and `activate` (stage 1: no config needed).

use std::process::ExitCode;

use crate::cli::{ActivateArgs, CommandsArgs};
use crate::env::EnvSnapshot;
use crate::error::Error;
use crate::ui::Ui;

/// The commands the wrapper intercepts, printed by `commands --sh`.
pub const SH_COMMANDS: [&str; 2] = ["cd", "on"];

/// Prints `CARGO_PKG_VERSION`.
///
/// # Errors
///
/// A broken pipe.
pub fn version(ui: &mut Ui) -> Result<ExitCode, Error> {
    let _ = ui;
    unimplemented!("W2.12")
}

/// `commands --sh`: prints [`SH_COMMANDS`], one per line, and nothing else.
///
/// # Errors
///
/// A broken pipe.
pub fn commands(ui: &mut Ui, args: CommandsArgs) -> Result<ExitCode, Error> {
    let _ = (ui, args);
    unimplemented!("W2.12")
}

/// `activate [shell]`: prints the embedded wrapper for the given or detected
/// shell.
///
/// # Errors
///
/// A broken pipe.
pub fn activate(env: &EnvSnapshot, ui: &mut Ui, args: ActivateArgs) -> Result<ExitCode, Error> {
    let _ = (env, ui, args);
    unimplemented!("W2.12")
}
