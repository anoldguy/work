//! `favorite [domain]`.

use std::process::ExitCode;

use crate::app::App;
use crate::cli::FavoriteArgs;
use crate::error::Error;

/// Picks the domain (the only one, or a picker), fetches member and extra
/// orgs under a progress title, runs `pick_many` with current favourites
/// preselected, and saves them (empty clears).
///
/// # Errors
///
/// `UiError::NotInteractive` without a TTY, unknown domain, token, API, or
/// config write failures.
pub fn run(app: &mut App, args: FavoriteArgs) -> Result<ExitCode, Error> {
    let _ = (app, args);
    unimplemented!("W5.4")
}
