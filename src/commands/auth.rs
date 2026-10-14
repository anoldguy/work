//! `auth login|logout|status`.

use std::process::ExitCode;

use crate::app::App;
use crate::cli::AuthArgs;
use crate::error::Error;

/// `login`/`logout` delegate to gh with stdio inherited. `status` prints one
/// line per effective domain and returns exit code 1, without an error line,
/// if any domain has no token.
///
/// # Errors
///
/// gh failures for `login`/`logout`; a broken pipe.
pub fn run(app: &mut App, args: AuthArgs) -> Result<ExitCode, Error> {
    let _ = (app, args);
    unimplemented!("W3.5")
}
