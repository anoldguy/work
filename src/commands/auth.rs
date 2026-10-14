//! `auth login|logout|status`.

use std::process::ExitCode;

use crate::app::App;
use crate::auth::{AuthError, Tokens};
use crate::cli::{AuthAction, AuthArgs};
use crate::domain::Domain;
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

/// `login`/`logout` delegate to gh with stdio inherited. `status` prints one
/// line per effective domain and returns exit code 1, without an error line,
/// if any domain has no token.
///
/// # Errors
///
/// gh failures for `login`/`logout`; a missing `gh` for `status`; a broken
/// pipe.
pub fn run(app: &mut App, args: AuthArgs) -> Result<ExitCode, Error> {
    let AuthArgs { action } = args;
    match action {
        AuthAction::Status => status(app),
        other => {
            login_logout(&*app.gh, &mut app.ui, &other)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn status(app: &mut App) -> Result<ExitCode, Error> {
    let domains = app.config.effective_domains();
    let tokens = Tokens::resolve(&*app.gh, &app.env.github_token, &domains);
    let mut lines = Vec::with_capacity(domains.len());
    let mut missing = false;
    for domain in &domains {
        match tokens.get(domain) {
            Ok((_, source)) => {
                lines.push(format!("{domain}: token found ({})", source.label()));
            }
            Err(AuthError::Gh(GhError::NotInstalled)) => {
                return Err(GhError::NotInstalled.into());
            }
            Err(err) => {
                missing = true;
                lines.push(failure_line(domain, &err));
            }
        }
    }
    for line in &lines {
        app.ui.stdout_line(line)?;
    }
    app.ui.flush()?;
    Ok(if missing {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn failure_line(domain: &Domain, err: &AuthError) -> String {
    match err {
        AuthError::NoToken { .. } => {
            format!("{domain}: no token (run 'gh auth login --hostname {domain}')")
        }
        AuthError::InvalidEnvToken => format!("{domain}: GITHUB_TOKEN is set but invalid"),
        AuthError::Gh(gh) => format!("{domain}: no token ({gh})"),
    }
}
