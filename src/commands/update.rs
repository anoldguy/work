//! `update [--all-orgs]`.

use std::collections::BTreeSet;
use std::process::ExitCode;

use crate::app::App;
use crate::cache::Cache;
use crate::cache::refresh::{Freshness, load_or_refresh};
use crate::cli::UpdateArgs;
use crate::error::Error;
use crate::text::plural;
use crate::ui::Ui;

/// Forces a refresh, then prints `Cached N repositories across D domains`.
///
/// # Errors
///
/// `CacheError::AllFailed` or store failures.
pub fn run(app: &App, ui: &mut Ui, args: &UpdateArgs) -> Result<ExitCode, Error> {
    let all_orgs = args.all_orgs;
    let freshness = Freshness::Force { all_orgs };
    let cache = load_or_refresh(app, ui, freshness)?;
    ui.stdout_line(&summary(&cache))?;
    Ok(ExitCode::SUCCESS)
}

/// `D` counts only domains that have cached repositories.
fn summary(cache: &Cache) -> String {
    let repos = cache.repos.len();
    let domains = cache
        .repos
        .iter()
        .map(|r| r.slug.domain())
        .collect::<BTreeSet<_>>()
        .len();
    format!(
        "Cached {repos} {} across {domains} {}",
        plural(repos, "repository", "repositories"),
        plural(domains, "domain", "domains"),
    )
}
