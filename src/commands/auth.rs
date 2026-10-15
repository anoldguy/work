//! `auth login|logout|status`, all delegated to `gh` with stdio inherited.

use std::process::ExitCode;

use crate::app::App;
use crate::domain::Domain;
use crate::error::Error;
use crate::gh::{Gh, GhError};
use crate::proc::Interactive;
use crate::ui::Ui;

/// `auth login`: `gh auth login --hostname <domain>` with stdio inherited.
/// Needs no config.
///
/// # Errors
///
/// gh failures; a broken pipe.
pub fn login(gh: &dyn Gh, ui: &mut Ui, domain: &Domain) -> Result<ExitCode, Error> {
    gh.login(&mut Interactive::new(ui)?, domain)?;
    Ok(ExitCode::SUCCESS)
}

/// `auth logout`: `gh auth logout --hostname <domain>` with stdio inherited.
/// Needs no config.
///
/// # Errors
///
/// gh failures; a broken pipe.
pub fn logout(gh: &dyn Gh, ui: &mut Ui, domain: &Domain) -> Result<ExitCode, Error> {
    gh.logout(&mut Interactive::new(ui)?, domain)?;
    Ok(ExitCode::SUCCESS)
}

/// `auth status`: runs `gh auth status --hostname <d>` for every effective
/// domain, letting gh print its own report, and returns exit code 1,
/// without an error line, if any domain is not logged in.
///
/// # Errors
///
/// A missing `gh`; a broken pipe.
pub fn status(app: &App, ui: &mut Ui) -> Result<ExitCode, Error> {
    let mut term = Interactive::new(ui)?;
    let mut failed = false;
    for domain in &app.config.effective_domains() {
        match app.gh.auth_status(&mut term, domain) {
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
