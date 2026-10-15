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
/// line (refreshing a stale cache first). Not called for `--complete`.
///
/// # Errors
///
/// Cache failures, or a broken pipe.
pub fn run(app: &mut App, args: &ListArgs) -> Result<ExitCode, Error> {
    let ListArgs {
        query,
        filter: substring,
        complete: _,
    } = args;
    let cache = load_or_refresh(app, Freshness::AutoUpdate)?;
    let lines: Vec<String> = match query {
        None => sorted_slugs(&cache.repos),
        Some(needle) if *substring => filter(&cache.repos, needle)
            .into_iter()
            .map(|i| cache.repos[i].slug.to_string())
            .collect(),
        Some(raw) => {
            let q = Query::parse(raw)?;
            let history = listed_history(app, &cache);
            let now = app.now;
            search(&cache.repos, &q, |s| history.frecency.score(s, now))
                .into_iter()
                .map(|r| cache.repos[r.index].slug.to_string())
                .collect()
        }
    };
    for line in &lines {
        app.ui.stdout_line(line)?;
    }
    Ok(ExitCode::SUCCESS)
}

/// Every slug, sorted.
fn sorted_slugs(repos: &[Repo]) -> Vec<String> {
    let mut slugs: Vec<&Slug> = repos.iter().map(|r| &r.slug).collect();
    slugs.sort();
    slugs.into_iter().map(ToString::to_string).collect()
}

/// `list --complete` (stage 1): loads the config and the cache without any
/// network, and prints de-duplicated `name` and `org/name` entries. Prints
/// nothing and exits 0 on any failure (missing or broken config or cache,
/// broken pipe), because a shell runs it on every Tab press.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorted_slugs_orders_by_slug() {
        let mk = |s: &str| -> Repo {
            serde_json::from_value(serde_json::json!({
                "slug": s,
                "clone_url": format!("https://{s}.git"),
                "html_url": format!("https://{s}"),
            }))
            .unwrap()
        };
        let repos = [mk("github.com/b/x"), mk("github.com/a/y")];
        assert_eq!(sorted_slugs(&repos), ["github.com/a/y", "github.com/b/x"]);
    }
}
