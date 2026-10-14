//! `update [--all-orgs]`.

use std::collections::BTreeSet;
use std::process::ExitCode;

use crate::app::App;
use crate::cache::Cache;
use crate::cache::refresh::{Freshness, load_or_refresh};
use crate::cli::UpdateArgs;
use crate::error::Error;
use crate::text::plural;

/// Forces a refresh, then prints `Cached N repositories across D domains`.
///
/// # Errors
///
/// `CacheError::AllFailed` or store failures.
pub fn run(app: &mut App, args: &UpdateArgs) -> Result<ExitCode, Error> {
    let all_orgs = args.all_orgs;
    let freshness = Freshness::Force { all_orgs };
    let cache = load_or_refresh(app, freshness)?;
    app.ui.stdout_line(&summary(&cache))?;
    Ok(ExitCode::SUCCESS)
}

/// `Cached N repositories across D domains` (D counts domains that have
/// cached repositories), with singular nouns for 1.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Repo, StoredTime};

    fn repo(name: &str) -> Repo {
        serde_json::from_value(serde_json::json!({
            "slug": format!("github.com/acme/{name}"),
            "clone_url": format!("https://github.com/acme/{name}.git"),
            "html_url": format!("https://github.com/acme/{name}"),
        }))
        .expect("repo")
    }

    fn cache(names: &[&str]) -> Cache {
        let now = StoredTime::from_unix_seconds(1_700_000_000).expect("time");
        Cache::new(names.iter().map(|n| repo(n)).collect(), now)
    }

    #[test]
    fn summary_pluralises() {
        assert_eq!(
            summary(&cache(&["a", "b"])),
            "Cached 2 repositories across 1 domain"
        );
        assert_eq!(
            summary(&cache(&["a"])),
            "Cached 1 repository across 1 domain"
        );
        assert_eq!(
            summary(&cache(&[])),
            "Cached 0 repositories across 0 domains"
        );
    }
}
