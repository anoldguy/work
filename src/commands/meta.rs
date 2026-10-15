//! `version` and `activate` (stage 1: no config needed).

use std::process::ExitCode;

use crate::cli::ActivateArgs;
use crate::env::EnvSnapshot;
use crate::error::Error;
use crate::shell::{activation_script, detect};
use crate::ui::Ui;

/// Prints `CARGO_PKG_VERSION`.
///
/// # Errors
///
/// A broken pipe.
pub fn version(ui: &mut Ui) -> Result<ExitCode, Error> {
    ui.stdout_line(env!("CARGO_PKG_VERSION"))?;
    Ok(ExitCode::SUCCESS)
}

/// `activate [shell]`: prints the embedded wrapper for the given or detected
/// shell.
///
/// # Errors
///
/// A broken pipe. An unknown shell name is rejected by the argument parser
/// before this runs.
pub fn activate(
    env: &EnvSnapshot,
    ui: &mut Ui,
    ActivateArgs { shell }: &ActivateArgs,
) -> Result<ExitCode, Error> {
    let shell = shell.unwrap_or_else(|| detect(env));
    // The embedded scripts end with exactly one newline, which `stdout_line`
    // supplies, so the output is the script byte-for-byte.
    ui.stdout_line(activation_script(shell).trim_end_matches('\n'))?;
    Ok(ExitCode::SUCCESS)
}
