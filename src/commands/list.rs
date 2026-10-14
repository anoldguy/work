//! `list [query] [--filter]`, and the hidden `list --complete`.

use std::process::ExitCode;

use crate::app::App;
use crate::cli::ListArgs;
use crate::config::Locations;
use crate::error::Error;
use crate::ui::Ui;

/// Prints every slug (sorted), ranked results, or filter results, one per
/// line (refreshing a stale cache first). Not called for `--complete`.
///
/// # Errors
///
/// Cache failures, or a broken pipe.
pub fn run(app: &mut App, args: ListArgs) -> Result<ExitCode, Error> {
    let _ = (app, args);
    unimplemented!("W5.2")
}

/// `list --complete` (stage 1): loads the config and the cache without any
/// network, and prints de-duplicated `name` and `org/name` entries. Prints
/// nothing and exits 0 on any failure (missing or broken config or cache,
/// broken pipe), because a shell runs it on every Tab press.
#[must_use]
pub fn complete(locations: &Locations, ui: &mut Ui) -> ExitCode {
    let _ = (locations, ui);
    unimplemented!("W5.2")
}
