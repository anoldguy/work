//! `setup [domain]`.

use std::process::ExitCode;

use crate::app::App;
use crate::cli::SetupArgs;
use crate::error::Error;

/// Prompts for `work_dir`, resolves tokens (offering `gh auth login` and
/// retrying on failure), adds the domain (as default if first), and offers an
/// initial update.
///
/// # Errors
///
/// `UiError::NotInteractive` without a TTY, gh, token, or config write
/// failures.
pub fn run(app: &mut App, args: SetupArgs) -> Result<ExitCode, Error> {
    let _ = (app, args);
    unimplemented!("W5.5")
}
