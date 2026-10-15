//! `favorite [domain]`.

use std::process::ExitCode;

use crate::app::App;
use crate::cli::FavoriteArgs;
use crate::commands::config::pick_domain;
use crate::config::{ConfigDoc, ConfigError};
use crate::domain::{Domain, Org};
use crate::error::Error;
use crate::text::plural;
use crate::ui::Ui;
use crate::ui::{Interaction, UiError};

const PROGRESS_TITLE: &str = "Fetching organizations";

/// What the picker produced.
#[derive(Debug, PartialEq, Eq)]
enum Choice {
    /// Nothing to pick from.
    NoOrgs,
    /// The user cancelled.
    Cancelled,
    /// The new favourites (empty clears them).
    Selected(Vec<Org>),
}

/// Lets the user pick a domain's favourite orgs, with the current ones
/// preselected, and saves them (empty clears).
///
/// # Errors
///
/// `UiError::NotInteractive` without a TTY, unknown domain, gh, or
/// config write failures.
pub fn run(app: &App, ui: &mut Ui, args: &FavoriteArgs) -> Result<ExitCode, Error> {
    if !ui.interact().is_interactive() {
        return Err(UiError::NotInteractive { what: "favorite" }.into());
    }
    let FavoriteArgs { domain } = args;
    let Some(domain) = select_domain(app, ui, domain.as_ref())? else {
        return Ok(ExitCode::SUCCESS);
    };
    let cfg = app
        .config
        .domains()
        .get(&domain)
        .ok_or_else(|| ConfigError::UnknownDomain {
            domain: domain.clone(),
        })?;

    let gh = &*app.gh;
    let mut orgs = ui.with_progress(PROGRESS_TITLE, |_| gh.member_orgs(&domain))?;
    orgs.extend_from_slice(&cfg.extra_orgs);

    match choose_favorites(ui.interact(), &domain, &orgs, &cfg.favorite_orgs)? {
        Choice::NoOrgs => {
            ui.warn(&format!("no organizations found for {domain}"));
        }
        Choice::Cancelled => {
            ui.stderr_line("favorites unchanged")?;
        }
        Choice::Selected(favorites) => {
            let mut doc = ConfigDoc::open(&app.locations.config_file)?;
            doc.set_favorites(&domain, &favorites)?;
            doc.save()?;
            let line = if favorites.is_empty() {
                format!("Cleared favorite organizations for {domain}")
            } else {
                let count = favorites.len();
                let noun = plural(count, "organization", "organizations");
                format!("Saved {count} favorite {noun} for {domain}")
            };
            ui.stdout_line(&line)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// The domain named, the only configured one, or the user's pick. `None`
/// when the picker was cancelled.
fn select_domain(
    app: &App,
    ui: &mut Ui,
    requested: Option<&Domain>,
) -> Result<Option<Domain>, Error> {
    if let Some(domain) = requested {
        return if app.config.domains().contains_key(domain) {
            Ok(Some(domain.clone()))
        } else {
            Err(ConfigError::UnknownDomain {
                domain: domain.clone(),
            }
            .into())
        };
    }
    let mut configured = app.config.domains().keys();
    match (configured.next(), configured.next()) {
        (None, _) => {
            // `effective_domains` is never empty: it falls back to the implicit domain.
            let domain = app
                .config
                .effective_domains()
                .into_iter()
                .next()
                .expect("effective_domains is never empty");
            Err(ConfigError::UnknownDomain { domain }.into())
        }
        (Some(only), None) => Ok(Some(only.clone())),
        _ => pick_domain(&app.config, ui, "favorite", "Domain"),
    }
}

/// Picks over the union of `orgs` and `current`, so a favourite the API no
/// longer lists is not silently dropped.
fn choose_favorites(
    interact: &mut dyn Interaction,
    domain: &Domain,
    orgs: &[Org],
    current: &[Org],
) -> Result<Choice, UiError> {
    let mut candidates: Vec<Org> = orgs.iter().chain(current).cloned().collect();
    candidates.sort();
    candidates.dedup();
    if candidates.is_empty() {
        return Ok(Choice::NoOrgs);
    }
    let names: Vec<&str> = candidates.iter().map(Org::as_str).collect();
    let selected: Vec<bool> = candidates.iter().map(|o| current.contains(o)).collect();
    let prompt = format!("Favorite organizations for {domain}");
    let Some(chosen) = interact.pick_many(&prompt, &names, &selected)? else {
        return Ok(Choice::Cancelled);
    };
    Ok(Choice::Selected(
        candidates
            .into_iter()
            .zip(chosen)
            .filter_map(|(org, keep)| keep.then_some(org))
            .collect(),
    ))
}
