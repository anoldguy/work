//! `setup [domain]`.

use std::path::Path;
use std::process::ExitCode;

use crate::app::App;
use crate::cache::refresh::{Freshness, load_or_refresh};
use crate::cli::SetupArgs;
use crate::config::locations::ExpandedPath;
use crate::config::{self, ConfigDoc};
use crate::domain::{Domain, Org};
use crate::error::Error;
use crate::gh::GhError;
use crate::proc::Interactive;
use crate::text::plural;
use crate::ui::Ui;
use crate::ui::UiError;

const DEFAULT_DOMAIN: &str = "github.com";

/// Prompts for `work_dir`, checks gh's login, adds the domain (as default if
/// first), and offers an initial update.
///
/// Running it again for a configured domain changes nothing but what the
/// prompts change. Existing comments in the config file are preserved.
///
/// # Errors
///
/// `UiError::NotInteractive` without a TTY, a missing gh, or config write
/// failures.
pub fn run(app: App, ui: &mut Ui, args: &SetupArgs) -> Result<ExitCode, Error> {
    if !ui.interact().is_interactive() {
        return Err(UiError::NotInteractive { what: "setup" }.into());
    }
    let fallback: Domain;
    let domain = if let Some(domain) = &args.domain {
        domain
    } else {
        fallback = DEFAULT_DOMAIN.parse()?;
        &fallback
    };

    let mut doc = ConfigDoc::open(&app.locations.config_file)?;
    let default_dir = display_dir(app.config.work_dir(), &app.locations.home);
    let typed = ask_work_dir(&app, ui, &default_dir)?;
    // An unchanged answer leaves an existing file's spelling alone.
    if typed != default_dir || !app.locations.config_file.exists() {
        doc.set_work_dir(&typed)?;
    }

    let logged_in = ensure_login(&app, ui, domain)?;

    // The first domain becomes the default; a later one never changes it.
    let is_first = app.config.domains().is_empty();
    doc.add_domain(domain, is_first)?;
    doc.save()?;
    let app = App {
        config: config::store::load(&app.locations)?,
        ..app
    };
    ui.stderr_line(&format!("Saved {}", app.locations.config_file.display()))?;

    if logged_in && ui.interact().confirm("Fetch repositories now?", true)? {
        let cache = load_or_refresh(&app, ui, Freshness::Force { all_orgs: false })?;
        let repos = cache.repos.len();
        let noun = plural(repos, "repository", "repositories");
        ui.stderr_line(&format!("Cached {repos} {noun}"))?;
    }
    Ok(ExitCode::SUCCESS)
}

/// Prompts until the answer is a valid `work_dir`, so what `setup` writes
/// always loads.
fn ask_work_dir(app: &App, ui: &mut Ui, default_dir: &str) -> Result<String, Error> {
    loop {
        let Some(typed) = ui.interact().input("Work directory", default_dir)? else {
            return Err(UiError::Interrupted.into());
        };
        let typed = typed.trim();
        let typed = if typed.is_empty() { default_dir } else { typed };
        match ExpandedPath::expand(typed, "work_dir", &app.locations.home) {
            Ok(_) => return Ok(typed.to_owned()),
            Err(err) => ui.stderr_line(&err.to_string())?,
        }
    }
}

/// `Ok(false)` when the domain should still be saved but gh could not reach
/// it.
fn ensure_login(app: &App, ui: &mut Ui, domain: &Domain) -> Result<bool, Error> {
    let mut check = check_login(app, ui, domain)?;
    if let Err(err) = &check {
        let question = format!("gh could not reach {domain} ({err}). Run 'gh auth login' now?");
        if ui.interact().confirm(&question, true)? {
            app.gh.login(&mut Interactive::new(ui)?, domain)?;
            check = check_login(app, ui, domain)?;
        }
    }
    match check {
        Ok(login) => {
            ui.stderr_line(&format!("Authenticated to {domain} as {login}"))?;
            Ok(true)
        }
        Err(err) => {
            ui.warn(&format!("gh is not logged in to {domain}: {err}"));
            Ok(false)
        }
    }
}

/// The inner `Err` is gh's own one-line reason; a missing or unstartable
/// `gh` is the outer error.
fn check_login(app: &App, ui: &mut Ui, domain: &Domain) -> Result<Result<Org, GhError>, Error> {
    let gh = &*app.gh;
    let result = ui.with_progress("Checking gh login", |_| gh.viewer(domain));
    match result {
        Err(err @ (GhError::NotInstalled | GhError::Spawn { .. })) => Err(err.into()),
        other => Ok(other),
    }
}

/// A path that is not UTF-8 has no faithful text form, so `~/code` is
/// offered instead of lossy text.
fn display_dir(dir: &Path, home: &Path) -> String {
    let shown = match dir.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => Some("~".to_owned()),
        Ok(rest) => rest.to_str().map(|r| format!("~/{r}")),
        Err(_) => dir.to_str().map(str::to_owned),
    };
    shown.unwrap_or_else(|| "~/code".to_owned())
}
