//! Resolving a query to a repo, cloning it if needed, and recording the
//! access.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::app::App;
use crate::cache::Cache;
use crate::core::paths::repo_path;
use crate::core::query_frecency::{Decision, decide};
use crate::core::resolve::resolve;
use crate::core::search::{Query, search};
use crate::domain::{Domain, QueryKey, Repo, Slug};
use crate::error::Error;
use crate::frecency::{self, History, Problem};
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

/// What [`ensure_cloned`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloneStatus {
    /// `.git` already existed.
    Already,
    /// It was cloned now.
    Cloned,
}

/// Resolves `query` against `cache`, ranking by frecency and showing the
/// picker on a TTY when needed (always when `pick` and several repos match).
///
/// # Errors
///
/// An invalid query; [`ResolveError::NotFound`]; [`ResolveError::Ambiguous`]
/// without a TTY or when the picker is cancelled; `UiError::NotInteractive`
/// for `--pick` without a TTY; `UiError::Interrupted` on Ctrl-C.
pub fn resolve_target(
    app: &App,
    ui: &mut Ui,
    cache: &Cache,
    query: &str,
    pick: bool,
) -> Result<Target, Error> {
    let query = Query::parse(query)?;
    let history = listed_history(app, ui, cache);
    let interactive = ui.interact().is_interactive();
    let now = app.now;
    let results = search(&cache.repos, &query, |s| history.frecency.score(s, now));
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
        Decision::NotFound => return Err(ResolveError::NotFound { query: key.clone() }.into()),
        Decision::Auto(index) => (index, true),
        Decision::Ask(candidates) => (ask(ui, cache, key, &candidates, interactive)?, true),
    };
    let mut target = target_for(app, cache, index);
    target.picked_for = picked.then(|| key.clone());
    Ok(target)
}

/// The frecency history restricted to the slugs `cache` lists. The file
/// keeps the other entries; they count again when their repo returns.
pub(crate) fn listed_history(app: &App, ui: &mut Ui, cache: &Cache) -> History {
    let (mut history, problem) = frecency::load(app.config.cache_dir());
    if let Some(problem) = problem {
        ui.warn(&problem.to_string());
    }
    let listed: HashSet<&Slug> = cache.repos.iter().map(|r| &r.slug).collect();
    history.frecency.retain(|s, _| listed.contains(s));
    history.query_frecency.retain(|s, _| listed.contains(s));
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
    #[error("repository not found: {query}")]
    NotFound {
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
