//! `forget <query>`.

use std::process::ExitCode;

use crate::adopted;
use crate::app::App;
use crate::cache::{CACHE_VERSION, Cache};
use crate::cli::ResolveArgs;
use crate::commands::resolve::{Reach, ResolveError, resolve_target};
use crate::error::Error;
use crate::ui::Ui;

/// Resolves `query` among the adopted repos only and removes the one it
/// names, printing `Forgot <slug>`. The clone and frecency stay.
///
/// # Errors
///
/// [`ResolveError::NotAdopted`], other resolution failures, or a state file
/// that cannot be written.
pub fn run(app: &App, ui: &mut Ui, args: &ResolveArgs) -> Result<ExitCode, Error> {
    let dir = app.config.cache_dir();
    let (adopted, problem) = adopted::load(dir);
    if let Some(problem) = problem {
        ui.warn(&problem.to_string());
    }
    let mut known = Cache {
        version: CACHE_VERSION,
        updated_at: app.now,
        repos: adopted.repos,
    };
    let target = resolve_target(app, ui, &mut known, &args.query, args.pick, Reach::Known)
        .map_err(|err| match err {
            Error::Resolve(ResolveError::NotFound { query, .. }) => {
                ResolveError::NotAdopted { query }.into()
            }
            other => other,
        })?;
    let slug = &target.repo(&known.repos).slug;
    let (_, problem) = adopted::update(dir, |a| a.remove(slug))?;
    if let Some(problem) = problem {
        ui.warn(&problem.to_string());
    }
    ui.stdout_line(&format!("Forgot {slug}"))?;
    Ok(ExitCode::SUCCESS)
}
