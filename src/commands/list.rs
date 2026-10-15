//! `list [query] [--filter]`, and the hidden `list --complete`.

use std::collections::BTreeSet;
use std::process::ExitCode;

use crate::app::App;
use crate::cache::refresh::{Freshness, load_or_refresh};
use crate::cache::store::{self, LoadOutcome};
use crate::cli::ListArgs;
use crate::commands::resolve::listed_history;
use crate::config::{Locations, store as config_store};
use crate::core::search::{Query, filter, search};
use crate::domain::{Repo, Slug};
use crate::error::Error;
use crate::ui::Ui;

/// Prints every slug (sorted), ranked results, or filter results, one per
/// line, refreshing a stale cache first.
///
/// # Errors
///
/// Cache failures, or a broken pipe.
pub fn run(app: &App, ui: &mut Ui, args: &ListArgs) -> Result<ExitCode, Error> {
    let ListArgs {
        query,
        filter: substring,
        complete: _,
    } = args;
    let cache = load_or_refresh(app, ui, Freshness::AutoUpdate)?;
    let lines: Vec<String> = match query {
        None => sorted_slugs(&cache.repos),
        Some(needle) if *substring => filter(&cache.repos, needle)
            .into_iter()
            .map(|i| cache.repos[i].slug.to_string())
            .collect(),
        Some(raw) => {
            let q = Query::parse(raw)?;
            let history = listed_history(app, ui, &cache);
            let now = app.now;
            search(&cache.repos, &q, |s| history.frecency.score(s, now))
                .into_iter()
                .map(|r| cache.repos[r.index].slug.to_string())
                .collect()
        }
    };
    for line in &lines {
        ui.stdout_line(line)?;
    }
    Ok(ExitCode::SUCCESS)
}

fn sorted_slugs(repos: &[Repo]) -> Vec<String> {
    let mut slugs: Vec<&Slug> = repos.iter().map(|r| &r.slug).collect();
    slugs.sort();
    slugs.into_iter().map(ToString::to_string).collect()
}

/// Prints de-duplicated `name` and `org/name` completion candidates from the
/// cache, without any network. Prints nothing and exits 0 on any failure,
/// because a shell runs it on every Tab press.
#[must_use]
pub fn complete(locations: &Locations, ui: &mut Ui) -> ExitCode {
    let Ok(config) = config_store::load(locations) else {
        return ExitCode::SUCCESS;
    };
    let LoadOutcome::Loaded(cache) = store::load(config.cache_dir()) else {
        return ExitCode::SUCCESS;
    };
    let entries: BTreeSet<String> = cache
        .repos
        .iter()
        .flat_map(|r| {
            let (org, name) = (r.slug.org(), r.slug.name());
            [name.to_string(), format!("{org}/{name}")]
        })
        .collect();
    for entry in &entries {
        if ui.stdout_line(entry).is_err() {
            break;
        }
    }
    ExitCode::SUCCESS
}
