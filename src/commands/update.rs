//! `update [--all-orgs]`.

use std::collections::BTreeSet;
use std::process::ExitCode;

use crate::adopted;
use crate::app::App;
use crate::cache::Cache;
use crate::cache::fetch::FETCH_JOBS;
use crate::cache::refresh::{Freshness, load_or_refresh};
use crate::cli::UpdateArgs;
use crate::commands::lookup::{Answer, ask};
use crate::domain::{Repo, Slug};
use crate::error::Error;
use crate::pool::run_bounded;
use crate::text::plural;
use crate::ui::Ui;

const RECHECK_TITLE: &str = "Checking adopted repositories";

/// Forces a refresh, re-checks the adopted repos, then prints `Cached N
/// repositories across D domains`.
///
/// # Errors
///
/// `CacheError::AllFailed` or store failures.
pub fn run(app: &App, ui: &mut Ui, args: &UpdateArgs) -> Result<ExitCode, Error> {
    let all_orgs = args.all_orgs;
    let freshness = Freshness::Force { all_orgs };
    let cache = load_or_refresh(app, ui, freshness)?;
    recheck_adopted(app, ui);
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

/// Looks every adopted repo on a configured domain up again: drops the ones
/// GitHub no longer finds and moves renamed ones to their current slug. Any
/// other answer keeps the repo. Problems are warnings.
fn recheck_adopted(app: &App, ui: &mut Ui) {
    let dir = app.config.cache_dir();
    let (adopted, problem) = adopted::load(dir);
    if let Some(problem) = problem {
        ui.warn(&problem.to_string());
    }
    let domains = app.config.effective_domains();
    let repos: Vec<Repo> = adopted
        .repos
        .into_iter()
        .filter(|r| domains.contains(r.slug.domain()))
        .collect();
    if repos.is_empty() {
        return;
    }
    let gh = &*app.gh;
    let answers = ui.with_progress(RECHECK_TITLE, |_| {
        run_bounded(&repos, FETCH_JOBS, |repo| {
            let slug = &repo.slug;
            ask(gh, slug.domain(), slug.org(), slug.name())
        })
    });
    let mut gone: Vec<&Slug> = Vec::new();
    let mut moved: Vec<(&Slug, Repo)> = Vec::new();
    for (repo, answer) in repos.iter().zip(answers) {
        match answer {
            Answer::NotFound => gone.push(&repo.slug),
            Answer::Found(current) if current.slug != repo.slug => {
                moved.push((&repo.slug, current));
            }
            Answer::Found(_) | Answer::Denied(_) | Answer::Failed(_) => {}
        }
    }
    if gone.is_empty() && moved.is_empty() {
        return;
    }
    let updated = adopted::update(dir, |a| {
        for slug in &gone {
            a.remove(slug);
        }
        for (old, current) in moved {
            if a.remove(old) {
                a.add(current);
            }
        }
    });
    match updated {
        Ok(((), problem)) => {
            if let Some(problem) = problem {
                ui.warn(&problem.to_string());
            }
            for slug in gone {
                ui.warn(&format!(
                    "dropped adopted repository {slug} (GitHub no longer finds it)"
                ));
            }
        }
        Err(err) => ui.warn(&err.to_string()),
    }
}
