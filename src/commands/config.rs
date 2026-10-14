//! `config [--edit] [path | default [domain]]`.
//!
//! `path` and `--edit` run in stage 1 with only `Locations`, so they work
//! (and `--edit` can repair the file) when the config is broken. The summary
//! and `default` run with an `App`.

use std::process::ExitCode;

use crate::app::App;
use crate::config::Locations;
use crate::domain::Domain;
use crate::env::EnvSnapshot;
use crate::error::Error;
use crate::ui::Ui;

/// `config path`: prints the config file's path and whether it exists.
///
/// # Errors
///
/// A broken pipe.
pub fn path(locations: &Locations, ui: &mut Ui) -> Result<ExitCode, Error> {
    let _ = (locations, ui);
    unimplemented!("W3.6")
}

/// `config --edit`: creates the config directory if needed and opens the
/// file in the editor chain (`$WORK_EDITOR`, `$EDITOR`, `vi`) with stdio
/// inherited.
///
/// # Errors
///
/// [`Error::CreateDir`] or an editor failure.
pub fn edit(env: &EnvSnapshot, locations: &Locations) -> Result<ExitCode, Error> {
    let _ = (env, locations);
    unimplemented!("W3.6")
}

/// `config`: prints a summary of the effective configuration.
///
/// # Errors
///
/// A broken pipe.
pub fn show(app: &mut App) -> Result<ExitCode, Error> {
    let _ = app;
    unimplemented!("W3.6")
}

/// `config default [domain]`: makes `domain` (or the one picked from the
/// configured domains) the default, keeping the file's comments.
///
/// # Errors
///
/// `ConfigError::UnknownDomain` or `NoDomains`, picker failures, or config
/// write failures.
pub fn set_default(app: &mut App, domain: Option<Domain>) -> Result<ExitCode, Error> {
    let _ = (app, domain);
    unimplemented!("W3.6")
}
