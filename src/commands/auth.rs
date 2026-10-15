//! `auth login|logout|status`, all delegated to `gh` with stdio inherited.

use std::process::ExitCode;

use crate::app::App;
use crate::cli::{AuthAction, AuthArgs};
use crate::error::Error;
use crate::gh::{Gh, GhError};
use crate::ui::Ui;

/// Runs the config-independent `login`/`logout` actions through `gh` with
/// stdio inherited, after flushing `ui`. Any other action is a no-op.
///
/// # Errors
///
/// gh failures; a broken pipe.
pub fn login_logout(gh: &dyn Gh, ui: &mut Ui, action: &AuthAction) -> Result<(), Error> {
    match action {
        AuthAction::Login(arg) => {
            ui.flush()?;
            gh.login(&arg.domain)?;
        }
        AuthAction::Logout(arg) => {
            ui.flush()?;
            gh.logout(&arg.domain)?;
        }
        AuthAction::Status => {}
    }
    Ok(())
}

/// `login`/`logout` delegate to gh with stdio inherited. `status` runs
/// `gh auth status --hostname <d>` for every effective domain, letting gh
/// print its own report, and returns exit code 1, without an error line, if
/// any domain is not logged in.
///
/// # Errors
///
/// gh failures for `login`/`logout`; a missing `gh`; a broken pipe.
pub fn run(app: &mut App, args: &AuthArgs) -> Result<ExitCode, Error> {
    let AuthArgs { action } = args;
    match action {
        AuthAction::Status => status(app),
        other => {
            login_logout(&*app.gh, &mut app.ui, other)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn status(app: &mut App) -> Result<ExitCode, Error> {
    let mut failed = false;
    for domain in &app.config.effective_domains() {
        // gh inherits stdio, so nothing of ours may still be buffered.
        app.ui.flush()?;
        match app.gh.auth_status(domain) {
            Ok(()) => {}
            Err(GhError::Failed { .. }) => failed = true,
            Err(err) => return Err(err.into()),
        }
    }
    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}
