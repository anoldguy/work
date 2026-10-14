//! `config [--edit] [path | default [domain]]`.
//!
//! `path` and `--edit` run in stage 1 with only `Locations`, so they work
//! (and `--edit` can repair the file) when the config is broken. The summary
//! and `default` run with an `App`.

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use crate::app::App;
use crate::config::{Config, ConfigDoc, ConfigError, Locations};
use crate::domain::{Domain, Org};
use crate::env::EnvSnapshot;
use crate::error::Error;
use crate::github::api_base;
use crate::platform::{self, EditorCmd};
use crate::ui::{Ui, UiError};

/// `config path`: prints the config file's path and whether it exists.
///
/// # Errors
///
/// A broken pipe.
pub fn path(locations: &Locations, ui: &mut Ui) -> Result<ExitCode, Error> {
    let file = &locations.config_file;
    ui.stdout_line(&file.display().to_string())?;
    ui.flush()?;
    ui.stderr_line(&format!("config file {}", file_state(file)))?;
    Ok(ExitCode::SUCCESS)
}

/// `exists` or `missing`.
fn file_state(file: &Path) -> &'static str {
    if file.exists() { "exists" } else { "missing" }
}

/// `config --edit`: creates the config directory if needed and opens the
/// file in the editor chain (`$WORK_EDITOR`, `$EDITOR`, `vi`) with stdio
/// inherited.
///
/// Prints nothing itself, so there is no buffered `Ui` output to flush
/// before the editor takes the terminal (the contract passes no `Ui`).
///
/// # Errors
///
/// [`Error::CreateDir`] or an editor failure.
pub fn edit(env: &EnvSnapshot, locations: &Locations) -> Result<ExitCode, Error> {
    let file = &locations.config_file;
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).map_err(|source| Error::CreateDir {
            path: dir.to_path_buf(),
            source,
        })?;
    }
    platform::spawn_editor(&EditorCmd::from_env(env), file)?;
    Ok(ExitCode::SUCCESS)
}

/// `config`: prints a summary of the effective configuration.
///
/// # Errors
///
/// A broken pipe.
pub fn show(app: &mut App) -> Result<ExitCode, Error> {
    let App {
        locations,
        config,
        ui,
        ..
    } = app;
    let file = &locations.config_file;
    let mut lines = vec![
        format!("config file: {} ({})", file.display(), file_state(file)),
        format!("work_dir:    {}", config.work_dir().display()),
        format!("cache_path:  {}", config.cache_dir().display()),
        "filters:".to_owned(),
        format!(
            "  include_orgs: {}",
            org_list(&config.filters().include_orgs)
        ),
        format!(
            "  exclude_orgs: {}",
            org_list(&config.filters().exclude_orgs)
        ),
    ];
    if config.domains().is_empty() {
        lines.push("domains: (none configured; using implicit github.com)".to_owned());
    } else {
        lines.push("domains:".to_owned());
        for (domain, cfg) in config.domains() {
            let marker = if cfg.is_default { " (default)" } else { "" };
            let api = match &cfg.api_url {
                Some(url) => url.to_string(),
                None => format!("{} (default)", api_base(domain, None)),
            };
            lines.push(format!("  {domain}{marker}"));
            lines.push(format!(
                "    favorite_orgs: {}",
                org_list(&cfg.favorite_orgs)
            ));
            lines.push(format!("    extra_orgs:    {}", org_list(&cfg.extra_orgs)));
            lines.push(format!("    api_url:       {api}"));
        }
    }
    for line in &lines {
        ui.stdout_line(line)?;
    }
    ui.flush()?;
    Ok(ExitCode::SUCCESS)
}

/// `config default [domain]`: makes `domain` (or the one picked from the
/// configured domains) the default, keeping the file's comments.
///
/// # Errors
///
/// `ConfigError::UnknownDomain` or `NoDomains`, picker failures, or config
/// write failures.
pub fn set_default(app: &mut App, domain: Option<Domain>) -> Result<ExitCode, Error> {
    let App {
        locations,
        config,
        ui,
        ..
    } = app;
    let picked = match domain {
        Some(domain) => Some(domain),
        None => pick_domain(config, ui)?,
    };
    let Some(chosen) = picked else {
        ui.stderr_line("no change")?;
        return Ok(ExitCode::SUCCESS);
    };
    let mut doc = ConfigDoc::open(&locations.config_file)?;
    doc.set_default(&chosen)?;
    doc.save()?;
    ui.stderr_line(&format!("default domain is now {chosen}"))?;
    Ok(ExitCode::SUCCESS)
}

/// Asks which configured domain to make the default; `None` when cancelled.
fn pick_domain(config: &Config, ui: &mut Ui) -> Result<Option<Domain>, Error> {
    let domains: Vec<&Domain> = config.domains().keys().collect();
    if domains.is_empty() {
        return Err(ConfigError::NoDomains.into());
    }
    let interact = ui.interact();
    if !interact.is_interactive() {
        return Err(UiError::NotInteractive {
            what: "config default without a domain",
        }
        .into());
    }
    let labels: Vec<&str> = domains.iter().map(|d| d.as_str()).collect();
    let picked = interact.pick_one("Default domain", &labels)?;
    Ok(picked
        .and_then(|index| domains.get(index))
        .map(|d| (*d).clone()))
}

/// `a, b, c`, or `(none)`.
fn org_list(orgs: &[Org]) -> String {
    if orgs.is_empty() {
        "(none)".to_owned()
    } else {
        orgs.iter().map(Org::as_str).collect::<Vec<_>>().join(", ")
    }
}
