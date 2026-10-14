//! `update [--all-orgs]`.

use std::process::ExitCode;

use crate::app::App;
use crate::cli::UpdateArgs;
use crate::error::Error;

/// Forces a refresh, then prints `Cached N repositories across D domains`.
///
/// # Errors
///
/// `CacheError::AllFailed` or store failures.
pub fn run(app: &mut App, args: UpdateArgs) -> Result<ExitCode, Error> {
    let _ = (app, args);
    unimplemented!("W5.4")
}
