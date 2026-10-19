//! Resolving a query to a repo, cloning it if needed, and recording the
//! access.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::adopted::{self, same_slug};
use crate::app::App;
use crate::cache::Cache;
use crate::cache::refresh::{Freshness, load_or_refresh};
use crate::commands::lookup::{Answer, Probe};
use crate::core::paths::repo_path;
use crate::core::query_frecency::{Decision, decide};
use crate::core::resolve::resolve;
use crate::core::search::{MatchTier, Query, SearchResult, search};
use crate::domain::{Domain, QueryKey, Repo, Slug};
use crate::error::Error;
use crate::frecency::{self, History, Problem};
use crate::state_file::StateError;
use crate::ui::Ui;
use crate::ui::UiError;

const PICK_PROMPT: &str = "Select a repository";

/// A resolved repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Index into `cache.repos`.
    pub index: usize,
    /// Where it lives (or will live) on disk.
    pub path: PathBuf,
    /// The query whose pick is saved to query frecency; `None` for a unique
    /// match.
    pub picked_for: Option<QueryKey>,
}

impl Target {
    /// The resolved repo in the cache's `repos`.
    ///
    /// # Panics
    ///
    /// If `self.index` is not an index into `repos` (a [`Target`] from
    /// [`resolve_target`] against the same cache always is).
    #[must_use]
    pub fn repo<'r>(&self, repos: &'r [Repo]) -> &'r Repo {
        &repos[self.index]
    }
}

/// Which repos a query can resolve to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Only the known repos it is given.
    Known,
    /// Also any repo the viewer can read, looked up on GitHub.
    Readable,
}

/// What [`ensure_cloned`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloneStatus {
    /// `.git` already existed.
    Already,
    /// It was cloned now.
    Cloned,
}

/// The cache, loaded or refreshed as `freshness` requires, with the
/// adopted repos on configured domains appended to `repos`. Never save the
/// result: the cache file holds listings only.
///
/// # Errors
///
/// As for [`load_or_refresh`].
pub fn load_known(app: &App, ui: &mut Ui, freshness: Freshness) -> Result<Cache, Error> {
    let mut cache = load_or_refresh(app, ui, freshness)?;
    let (adopted, problem) = adopted::load(app.config.cache_dir());
    if let Some(problem) = problem {
        ui.warn(&problem.to_string());
    }
    adopted::extend_known(&mut cache.repos, adopted, &app.config.effective_domains());
    Ok(cache)
}

/// Resolves `query` against `cache`, ranking by frecency and showing the
/// picker on a TTY when needed (always when `pick` and several repos match).
///
/// With [`Reach::Readable`], an `org/repo` or `domain/org/repo` query with no
/// exact match is looked up on GitHub first; a repo found there is appended
/// to `cache.repos`, and adopted once selected.
///
/// # Errors
///
/// An invalid query; [`ResolveError::NotFound`], [`ResolveError::Denied`] or
/// [`ResolveError::LookupFailed`]; [`ResolveError::Ambiguous`] without a TTY
/// or when the picker is cancelled; `UiError::NotInteractive` for `--pick`
/// without a TTY; `UiError::Interrupted` on Ctrl-C.
pub fn resolve_target(
    app: &App,
    ui: &mut Ui,
    cache: &mut Cache,
    query: &str,
    pick: bool,
    reach: Reach,
) -> Result<Target, Error> {
    let query = Query::parse(query)?;
    let mut history = load_history(app, ui);
    let interactive = ui.interact().is_interactive();
    let now = app.now;
    let known = cache.repos.len();
    let mut results = search(&cache.repos, &query, |s| history.frecency.score(s, now));
    let probe = match reach {
        Reach::Readable if !results.iter().any(|r| r.tier == MatchTier::Exact) => {
            Probe::plan(&app.config, &query)
        }
        _ => None,
    };
    if let Some(probe) = probe {
        let gh = &*app.gh;
        let title = format!("looking up {}", query.key());
        let mut found = Vec::new();
        let mut misses = Vec::new();
        for (domain, answer) in ui.with_progress(&title, |_| probe.run(gh)) {
            match answer {
                Answer::Found(repo) => found.push(repo),
                miss => misses.push((domain, miss)),
            }
        }
        if !found.is_empty() {
            results = found_results(cache, found, |s| history.frecency.score(s, now));
        } else if results.is_empty() || misses.iter().any(|(_, a)| matches!(a, Answer::Denied(_))) {
            return Err(lookup_error(&probe, &query, misses).into());
        } else {
            warn_failed(ui, &probe, &misses);
        }
    }
    restrict(&mut history, &cache.repos);
    let key = query.key();
    let decision = if pick && results.len() > 1 {
        if !interactive {
            return Err(UiError::NotInteractive { what: "--pick" }.into());
        }
        Decision::Ask(results.iter().map(|r| r.index).collect())
    } else {
        decide(
            &cache.repos,
            resolve(results),
            &history.query_frecency,
            key,
            now,
        )
    };
    let (index, picked) = match decision {
        Decision::Unique(index) => (index, false),
        Decision::NotFound => {
            return Err(ResolveError::NotFound {
                query: key.clone(),
                looked_up: Vec::new(),
            }
            .into());
        }
        Decision::Auto(index) => (index, true),
        Decision::Ask(candidates) => (ask(ui, cache, key, &candidates, interactive)?, true),
    };
    if index >= known {
        adopt(app, ui, &cache.repos[index]);
    }
    let mut target = target_for(app, cache, index);
    target.picked_for = picked.then(|| key.clone());
    Ok(target)
}

/// The repos found on GitHub as exact results, best first, appending to
/// `cache.repos` those it does not already hold.
fn found_results(
    cache: &mut Cache,
    found: Vec<Repo>,
    frecency: impl Fn(&Slug) -> f64,
) -> Vec<SearchResult> {
    let mut results: Vec<SearchResult> = Vec::new();
    for repo in found {
        let held = cache
            .repos
            .iter()
            .position(|r| same_slug(&r.slug, &repo.slug));
        let index = held.unwrap_or_else(|| {
            cache.repos.push(repo);
            cache.repos.len() - 1
        });
        if results.iter().all(|r| r.index != index) {
            let tier = MatchTier::Exact;
            let score = tier.score() + frecency(&cache.repos[index].slug);
            results.push(SearchResult { index, tier, score });
        }
    }
    results.sort_by(|a, b| b.score.total_cmp(&a.score));
    results
}

/// Why a lookup that found nothing fails: refused access first, then any
/// other failure, else not found. `answers` holds each looked-up domain's
/// answer, none of them `Found`, in domain order.
fn lookup_error(probe: &Probe, query: &Query, answers: Vec<(Domain, Answer)>) -> ResolveError {
    let mut failed = None;
    for (domain, answer) in answers {
        match answer {
            Answer::Denied(detail) => {
                return ResolveError::Denied {
                    slug: probe.slug_on(&domain),
                    detail: gh_message(&detail),
                    domain,
                };
            }
            Answer::Failed(detail) if failed.is_none() => failed = Some((domain, detail)),
            Answer::Found(_) | Answer::NotFound | Answer::Failed(_) => {}
        }
    }
    match failed {
        Some((domain, detail)) => ResolveError::LookupFailed {
            slug: probe.slug_on(&domain),
            detail: gh_message(&detail),
        },
        None => ResolveError::NotFound {
            query: query.key().clone(),
            looked_up: probe.domains().to_vec(),
        },
    }
}

/// One warning per lookup that failed for a reason other than "not found".
fn warn_failed(ui: &mut Ui, probe: &Probe, answers: &[(Domain, Answer)]) {
    for (domain, answer) in answers {
        if let Answer::Denied(detail) | Answer::Failed(detail) = answer {
            let slug = probe.slug_on(domain);
            ui.warn(&format!("could not look up {slug}: {}", gh_message(detail)));
        }
    }
}

/// gh's message without its `gh: ` prefix.
fn gh_message(detail: &str) -> String {
    detail.strip_prefix("gh: ").unwrap_or(detail).to_owned()
}

/// Adds `repo` to the adopted repos, saying so the first time. Failures are
/// warnings: the repo still resolves for this run.
fn adopt(app: &App, ui: &mut Ui, repo: &Repo) {
    match adopted::update(app.config.cache_dir(), |a| a.add(repo.clone())) {
        Ok((added, problem)) => {
            if let Some(problem) = problem {
                ui.warn(&problem.to_string());
            }
            if added {
                let _ = ui.stderr_line(&format!("adopted {}", repo.slug));
            }
        }
        // Loading the known repos already warned about these.
        Err(StateError::ReadOnly(_)) => {}
        Err(err) => ui.warn(&err.to_string()),
    }
}

/// The frecency history, warning once about a problem with the file.
fn load_history(app: &App, ui: &mut Ui) -> History {
    let (history, problem) = frecency::load(app.config.cache_dir());
    if let Some(problem) = problem {
        ui.warn(&problem.to_string());
    }
    history
}

/// Drops the entries for slugs `repos` does not hold. The file keeps them;
/// they count again when their repo returns.
fn restrict(history: &mut History, repos: &[Repo]) {
    let listed: HashSet<&Slug> = repos.iter().map(|r| &r.slug).collect();
    history.frecency.retain(|s, _| listed.contains(s));
    history.query_frecency.retain(|s, _| listed.contains(s));
}

/// The frecency history restricted to the slugs `cache` holds.
pub(crate) fn listed_history(app: &App, ui: &mut Ui, cache: &Cache) -> History {
    let mut history = load_history(app, ui);
    restrict(&mut history, &cache.repos);
    history
}

fn ask(
    ui: &mut Ui,
    cache: &Cache,
    key: &QueryKey,
    candidates: &[usize],
    interactive: bool,
) -> Result<usize, Error> {
    let slugs: Vec<&Slug> = candidates.iter().map(|&i| &cache.repos[i].slug).collect();
    let ambiguous = || ResolveError::Ambiguous {
        query: key.clone(),
        candidates: slugs.iter().map(|&s| s.clone()).collect(),
    };
    if !interactive {
        return Err(ambiguous().into());
    }
    let labels: Vec<String> = slugs.iter().map(ToString::to_string).collect();
    let items: Vec<&str> = labels.iter().map(String::as_str).collect();
    match ui.interact().pick_one(PICK_PROMPT, &items)? {
        Some(chosen) => candidates
            .get(chosen)
            .copied()
            .ok_or_else(|| ambiguous().into()),
        None => Err(ambiguous().into()),
    }
}

fn target_for(app: &App, cache: &Cache, index: usize) -> Target {
    let slug = &cache.repos[index].slug;
    let path = repo_path(app.config.work_dir(), app.config.default_domain(), slug);
    Target {
        index,
        path,
        picked_for: None,
    }
}

/// Whether `path` contains `.git`: a directory, or a file as in a worktree.
#[must_use]
pub(crate) fn is_cloned(path: &Path) -> bool {
    path.join(".git").exists()
}

/// `org/name` for the default domain, the full slug otherwise.
pub(crate) fn short_name(slug: &Slug, default: Option<&Domain>) -> String {
    if default == Some(slug.domain()) {
        format!("{}/{}", slug.org(), slug.name())
    } else {
        slug.to_string()
    }
}

/// Clones the target unless `.git` exists. When the clone fails, the parent
/// directories it created are removed again if still empty.
///
/// # Errors
///
/// [`Error::CreateDir`] or the git failure.
///
/// # Panics
///
/// If `target` was not resolved against `cache`.
pub fn ensure_cloned(
    app: &App,
    ui: &mut Ui,
    cache: &Cache,
    target: &Target,
) -> Result<CloneStatus, Error> {
    if is_cloned(&target.path) {
        return Ok(CloneStatus::Already);
    }
    let repo = target.repo(&cache.repos);
    let domain = repo.slug.domain();
    let created = match target.path.parent() {
        Some(parent) => {
            let missing = missing_dirs(parent);
            std::fs::create_dir_all(parent).map_err(|source| Error::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
            missing
        }
        None => Vec::new(),
    };
    let title = format!(
        "cloning {}",
        short_name(&repo.slug, app.config.default_domain())
    );
    let git = &*app.git;
    let dest = target.path.as_path();
    let cloned = ui.with_progress(&title, |_| git.clone_repo(&repo.clone_url, dest, domain));
    if cloned.is_err() {
        // Best effort: `remove_dir` refuses a directory that is not empty,
        // so nothing that was already there (or appeared since) is lost.
        for dir in &created {
            let _ = std::fs::remove_dir(dir);
        }
    }
    cloned?;
    Ok(CloneStatus::Cloned)
}

/// The directories `create_dir_all(dir)` would create, deepest first.
fn missing_dirs(dir: &Path) -> Vec<PathBuf> {
    dir.ancestors()
        .take_while(|d| !d.as_os_str().is_empty() && !d.exists())
        .map(Path::to_path_buf)
        .collect()
}

/// Records the access to the target, and its query pick if any. Failures
/// are warnings, since the command itself has already succeeded.
///
/// # Panics
///
/// If `target` was not resolved against `cache`.
pub fn record_access(app: &App, ui: &mut Ui, cache: &Cache, target: &Target) {
    let now = app.now;
    let slug = &target.repo(&cache.repos).slug;
    let recorded = frecency::record(app.config.cache_dir(), now, |history| {
        history.frecency.record(slug, now);
        if let Some(key) = &target.picked_for {
            history.query_frecency.record(key, slug, now);
        }
    });
    // Loading for resolution already warned about these.
    match recorded {
        Ok(None | Some(Problem::UnknownVersion { .. } | Problem::Unreadable { .. })) => {}
        Ok(Some(problem)) => ui.warn(&problem.to_string()),
        Err(err) => ui.warn(&err.to_string()),
    }
}

/// A resolution failure.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    /// No repo matched.
    #[error("repository not found: {query}{}", access_hint(.looked_up))]
    NotFound {
        /// The normalised query.
        query: QueryKey,
        /// The domains GitHub was asked, in order; empty when it was not.
        looked_up: Vec<Domain>,
    },
    /// GitHub refused access (HTTP 401 or 403).
    #[error(
        "cannot read {slug}: {detail} (check 'gh auth status --hostname {domain}'{})",
        sso_hint(.detail)
    )]
    Denied {
        /// The repo asked for.
        slug: Slug,
        /// The domain that refused.
        domain: Domain,
        /// gh's message, one line.
        detail: String,
    },
    /// Asking GitHub failed for another reason.
    #[error("cannot look up {slug}: {detail}")]
    LookupFailed {
        /// The repo asked for.
        slug: Slug,
        /// gh's message, one line.
        detail: String,
    },
    /// `forget` matched no adopted repo.
    #[error("no adopted repository matches: {query}")]
    NotAdopted {
        /// The normalised query.
        query: QueryKey,
    },
    /// Several repos matched and nobody picked.
    #[error("ambiguous query '{query}' (use a more specific query, or run in a terminal to pick)")]
    Ambiguous {
        /// The normalised query.
        query: QueryKey,
        /// Every candidate, in rank order.
        candidates: Vec<Slug>,
    },
    /// `pull` on a repo that has not been cloned.
    #[error("{repo} is not cloned (run 'work clone {repo}')")]
    NotCloned {
        /// The repo as shown to the user, e.g. `acme/foo`.
        repo: String,
    },
}

/// ` (if it exists, check ...)` naming the one domain looked up, or plain
/// `gh auth status` for several; empty when nothing was looked up.
fn access_hint(looked_up: &[Domain]) -> String {
    match looked_up {
        [] => String::new(),
        [domain] => {
            format!(" (if it exists, check you can read it: gh auth status --hostname {domain})")
        }
        _ => " (if it exists, check you can read it: gh auth status)".to_owned(),
    }
}

/// The SAML SSO part of the access hint, when gh's message is about SSO.
fn sso_hint(detail: &str) -> &'static str {
    if detail.contains("SAML") {
        "; for SAML SSO, authorize the token for the organization"
    } else {
        ""
    }
}
