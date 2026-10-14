//! The shared pipeline behind `cd`, `on`, `edit`, `clone`, `browse`, `open`
//! and `pull`: resolve a query, clone if needed, record the access.

use std::path::{Path, PathBuf};

use crate::app::App;
use crate::auth::Tokens;
use crate::cache::{Cache, store};
use crate::core::paths::repo_path;
use crate::core::query_frecency::{Decision, decide};
use crate::core::resolve::resolve;
use crate::core::search::{Query, search};
use crate::domain::{Domain, QueryKey, Repo, Slug};
use crate::error::Error;
use crate::ui::UiError;

/// The picker's prompt for an ambiguous query.
const PICK_PROMPT: &str = "Select a repository";

/// A resolved repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Index into `cache.repos`.
    pub index: usize,
    /// Where it lives (or will live) on disk.
    pub path: PathBuf,
    /// The query whose pick [`record_access`] saves to query frecency: set
    /// after an automatic answer or a picker choice, `None` for a unique
    /// match.
    pub picked_for: Option<QueryKey>,
}

impl Target {
    /// The resolved repo in `repos` (the cache's `repos`).
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

/// Resolves `query` against `cache` (search, `decide`, then the picker on a
/// TTY, always when `pick` and several repos match), recording query
/// frecency after an automatic answer or a pick.
///
/// # Errors
///
/// An invalid query; [`ResolveError::NotFound`]; [`ResolveError::Ambiguous`]
/// without a TTY or when the picker is cancelled; `UiError::NotInteractive`
/// for `--pick` without a TTY when several repos match; `UiError::Interrupted` on Ctrl-C.
pub fn resolve_target(
    app: &mut App,
    cache: &mut Cache,
    query: &str,
    pick: bool,
) -> Result<Target, Error> {
    let query = Query::parse(query)?;
    let interactive = app.ui.interact().is_interactive();
    let now = app.now;
    let results = search(&cache.repos, &query, |s| cache.frecency.score(s, now));
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
            &cache.query_frecency,
            key,
            now,
        )
    };
    let (index, picked) = match decision {
        Decision::Unique(index) => (index, false),
        Decision::NotFound => return Err(ResolveError::NotFound { query: key.clone() }.into()),
        Decision::Auto(index) => (index, true),
        Decision::Ask(candidates) => (ask(app, cache, key, &candidates, interactive)?, true),
    };
    if picked {
        cache
            .query_frecency
            .record(key, &cache.repos[index].slug, now);
    }
    let mut target = target_for(app, cache, index);
    target.picked_for = picked.then(|| key.clone());
    Ok(target)
}

/// Shows the picker over `candidates` (repo indices, rank order) and returns
/// the chosen repo index.
///
/// # Errors
///
/// [`ResolveError::Ambiguous`] without a terminal or when cancelled; the
/// picker's own failure (e.g. `UiError::Interrupted`).
fn ask(
    app: &mut App,
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
    match app.ui.interact().pick_one(PICK_PROMPT, &items)? {
        Some(chosen) => candidates
            .get(chosen)
            .copied()
            .ok_or_else(|| ambiguous().into()),
        None => Err(ambiguous().into()),
    }
}

/// The [`Target`] for repo `index`: where it lives under the work directory.
fn target_for(app: &App, cache: &Cache, index: usize) -> Target {
    let slug = &cache.repos[index].slug;
    let path = repo_path(app.config.work_dir(), app.config.default_domain(), slug);
    Target {
        index,
        path,
        picked_for: None,
    }
}

/// Whether the repo directory at `path` is a clone: it contains `.git` (a
/// directory, or a file as in a worktree). The one check `clone`, `pull`
/// and `sync` share.
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

/// Clones the target unless `.git` exists: resolves a token for the repo's
/// domain, creates the parent directories, and runs `git clone` under a
/// progress title. With no token for the domain, git clones without auth
/// (public repos, ssh `insteadOf`, credential helpers). When the clone
/// fails, the parent directories it created are removed again (if still
/// empty).
///
/// # Errors
///
/// [`Error::CreateDir`], an invalid `GITHUB_TOKEN`
/// ([`AuthError::InvalidEnvToken`]), a stdout flush failure, or the git
/// failure.
///
/// # Panics
///
/// If `target.index` is not an index into `cache.repos` (a [`Target`] from
/// [`resolve_target`] against the same cache always is).
pub fn ensure_cloned(app: &mut App, cache: &Cache, target: &Target) -> Result<CloneStatus, Error> {
    if is_cloned(&target.path) {
        return Ok(CloneStatus::Already);
    }
    let repo = target.repo(&cache.repos);
    let domain = repo.slug.domain();
    let tokens = Tokens::resolve(&*app.gh, &app.env.github_token, [domain]);
    let auth = tokens.git_auth(domain)?;
    // Pending stdout must land before git (which may inherit it) runs.
    app.ui.flush()?;
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
    let cloned = app.ui.with_progress(&title, |_| {
        git.clone_repo(&repo.clone_url, dest, auth.as_ref())
    });
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

/// Records repo frecency for the target and saves the cache. Called once the
/// command's action has succeeded.
///
/// `cache` gets the repo access (its query pick was recorded by
/// [`resolve_target`]). The save goes through [`store::update`]: under the
/// cache lock, the access and the query pick are re-applied to the newest
/// copy on disk, so a refresh or another access saved meanwhile is kept. If
/// that copy no longer lists the repo, nothing is recorded in it; if there
/// is no readable copy, `cache` is written.
///
/// # Errors
///
/// Saving the cache failed.
///
/// # Panics
///
/// If `target.index` is not an index into `cache.repos` (a [`Target`] from
/// [`resolve_target`] against the same cache always is).
pub fn record_access(app: &App, cache: &mut Cache, target: &Target) -> Result<(), Error> {
    let Cache {
        repos, frecency, ..
    } = cache;
    let now = app.now;
    frecency.record(&target.repo(repos).slug, now);
    let cache = &*cache;
    let slug = &target.repo(&cache.repos).slug;
    store::update(app.config.cache_dir(), |latest| {
        let Some(mut latest) = latest else {
            // Nothing readable on disk: write the caller's copy.
            return cache.clone();
        };
        if latest.repos.iter().any(|r| &r.slug == slug) {
            latest.frecency.record(slug, now);
            if let Some(key) = &target.picked_for {
                latest.query_frecency.record(key, slug, now);
            }
        }
        latest
    })?;
    Ok(())
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
    /// Several repos matched and nobody picked. A list error: `candidates`
    /// are printed as `  - <slug>` lines after the summary.
    #[error("ambiguous query '{query}' (use a more specific query, or run in a terminal to pick)")]
    Ambiguous {
        /// The normalised query.
        query: QueryKey,
        /// Every candidate (at most `RESULT_LIMIT`), in rank order.
        candidates: Vec<Slug>,
    },
    /// `pull` on a repo that has not been cloned.
    #[error("{repo} is not cloned (run 'work clone {repo}')")]
    NotCloned {
        /// The repo as shown to the user, e.g. `acme/foo`.
        repo: String,
    },
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::app::Adapters;
    use crate::auth::{AuthError, EnvToken, GitAuth};
    use crate::config::{self, Locations};
    use crate::core::sync_outcome::{RepoFacts, Upstream};
    use crate::domain::{CloneUrl, HtmlUrl, Repo, StoredTime};
    use crate::env::EnvSnapshot;
    use crate::git::{Git, GitError};
    use crate::testing::interact::PromptCall;
    use crate::testing::{
        Answer, FakeGh, FakeGit, GitOp, NullGitHub, PromptLog, RepoScript, ScriptedInteraction,
        SharedBuf,
    };
    use crate::ui::Ui;

    const NOW: i64 = 1_700_000_000;

    /// Lets a test keep a handle on the `FakeGit` it gives the `App`, and
    /// snapshots what stdout held when each clone started.
    #[derive(Debug)]
    struct SharedGit {
        inner: Arc<FakeGit>,
        out: SharedBuf,
        out_at_clone: Arc<Mutex<Vec<String>>>,
    }

    impl Git for SharedGit {
        fn clone_repo(
            &self,
            url: &CloneUrl,
            dest: &Path,
            auth: Option<&GitAuth<'_>>,
        ) -> Result<(), GitError> {
            self.out_at_clone
                .lock()
                .expect("lock")
                .push(self.out.contents());
            self.inner.clone_repo(url, dest, auth)
        }
        fn fetch(&self, repo: &Path, auth: Option<&GitAuth<'_>>) -> Result<(), GitError> {
            self.inner.fetch(repo, auth)
        }
        fn facts(&self, repo: &Path) -> Result<RepoFacts, GitError> {
            self.inner.facts(repo)
        }
        fn fast_forward(&self, repo: &Path) -> Result<(), GitError> {
            self.inner.fast_forward(repo)
        }
    }

    struct Harness {
        _dir: tempfile::TempDir,
        app: App,
        prompts: PromptLog,
        out: SharedBuf,
        out_at_clone: Arc<Mutex<Vec<String>>>,
    }

    /// An `App` whose home is `dir` (so the work dir is `dir/code`), with a
    /// `Plain` Ui over in-memory writers.
    fn harness_in(
        dir: tempfile::TempDir,
        interaction: ScriptedInteraction,
        git: Arc<FakeGit>,
        gh: FakeGh,
    ) -> Harness {
        let env = EnvSnapshot {
            home: Some(dir.path().to_path_buf()),
            ..EnvSnapshot::default()
        };
        let locations = Locations::from_env(&env).expect("locations");
        let config = config::store::load(&locations).expect("config");
        let prompts = interaction.log();
        let out = SharedBuf::new();
        let out_at_clone = Arc::default();
        let ui = Ui::for_test(
            Box::new(out.clone()),
            Box::new(SharedBuf::new()),
            Box::new(interaction),
        );
        let adapters = Adapters {
            git: Box::new(SharedGit {
                inner: git,
                out: out.clone(),
                out_at_clone: Arc::clone(&out_at_clone),
            }),
            gh: Box::new(gh),
            api: Box::new(NullGitHub),
        };
        let app = App::with_parts(env, locations, config, ui, adapters, now());
        Harness {
            _dir: dir,
            app,
            prompts,
            out,
            out_at_clone,
        }
    }

    fn harness(interactive: bool, answers: Vec<Answer>) -> Harness {
        harness_in(
            tempfile::tempdir().expect("tempdir"),
            ScriptedInteraction::new(interactive, answers),
            Arc::new(FakeGit::new()),
            FakeGh::new(),
        )
    }

    fn plain() -> Harness {
        harness(false, vec![])
    }

    /// A non-interactive harness whose `gh` has a token for github.com.
    fn cloning(git: Arc<FakeGit>) -> Harness {
        harness_in(
            tempfile::tempdir().expect("tempdir"),
            ScriptedInteraction::new(false, vec![]),
            git,
            FakeGh::new().with_token(github(), "tok-123"),
        )
    }

    fn now() -> StoredTime {
        StoredTime::from_unix_seconds(NOW).expect("time")
    }

    fn github() -> Domain {
        "github.com".parse().expect("domain")
    }

    fn repo(slug: &str) -> Repo {
        let slug = Slug::try_from(slug.to_owned()).expect("slug");
        let url = format!("https://{}/{}/{}", slug.domain(), slug.org(), slug.name());
        Repo {
            clone_url: CloneUrl::try_from(format!("{url}.git")).expect("clone url"),
            html_url: HtmlUrl::try_from(url).expect("html url"),
            slug,
        }
    }

    /// `foob` matches `foobar` and `foobaz` ambiguously; `api` is unique.
    fn cache() -> Cache {
        let repos = [
            "github.com/acme/foobar",
            "github.com/acme/foobaz",
            "ghe.corp.com/corp/api",
        ]
        .map(repo)
        .into();
        Cache::new(repos, now())
    }

    fn key(raw: &str) -> QueryKey {
        QueryKey::new(raw).expect("key")
    }

    fn query_count(cache: &Cache, query: &str, index: usize) -> u32 {
        cache
            .query_frecency
            .get(&key(query))
            .and_then(|f| f.get(&cache.repos[index].slug))
            .map_or(0, |d| d.access_count)
    }

    /// Gives `foob` a dominant history for `foobaz`.
    fn with_dominant_history(cache: &mut Cache) {
        let slug = cache.repos[1].slug.clone();
        for _ in 0..3 {
            cache.query_frecency.record(&key("foob"), &slug, now());
        }
    }

    #[test]
    fn a_clean_resolve_returns_the_repo_path_and_records_nothing() {
        let mut h = plain();
        let mut cache = cache();
        let target = resolve_target(&mut h.app, &mut cache, "foobar", false).expect("ok");
        assert_eq!(target.index, 0);
        assert_eq!(target.path, h.app.config.work_dir().join("acme/foobar"));
        assert!(cache.query_frecency.is_empty());
        assert_eq!(h.prompts.calls(), []);
    }

    #[test]
    fn a_non_default_domain_keeps_its_domain_segment() {
        let mut h = plain();
        let target = resolve_target(&mut h.app, &mut cache(), "api", false).expect("ok");
        assert_eq!(
            target.path,
            h.app.config.work_dir().join("ghe.corp.com/corp/api")
        );
    }

    #[test]
    fn an_invalid_query_is_a_name_error() {
        let mut h = plain();
        let err = resolve_target(&mut h.app, &mut cache(), "a/b/c/d", false).expect_err("invalid");
        assert_matches!(err, Error::Name(_));
    }

    #[test]
    fn no_match_is_not_found() {
        let mut h = plain();
        let err = resolve_target(&mut h.app, &mut cache(), "zzzz", false).expect_err("none");
        assert_matches!(
            err,
            Error::Resolve(ResolveError::NotFound { ref query }) if query.as_str() == "zzzz"
        );
    }

    #[test]
    fn ambiguity_without_a_tty_is_a_list_error_of_full_slugs() {
        let mut h = plain();
        let mut cache = cache();
        let err = resolve_target(&mut h.app, &mut cache, "foob", false).expect_err("ambiguous");
        assert_eq!(
            err.list_items(),
            ["github.com/acme/foobar", "github.com/acme/foobaz"]
        );
        assert_matches!(err, Error::Resolve(ResolveError::Ambiguous { .. }));
        assert!(!err.summary().contains('\n'));
        assert!(cache.query_frecency.is_empty());
    }

    #[test]
    fn ambiguity_on_a_tty_shows_the_picker_and_records_the_pick() {
        let mut h = harness(true, vec![Answer::Pick(Some(1))]);
        let mut cache = cache();
        let target = resolve_target(&mut h.app, &mut cache, "foob", false).expect("pick");
        assert_eq!(target.index, 1);
        assert_matches!(
            h.prompts.calls().as_slice(),
            [PromptCall::PickOne { items, .. }]
                if items == &["github.com/acme/foobar", "github.com/acme/foobaz"]
        );
        assert_eq!(query_count(&cache, "foob", 1), 1);
        assert_eq!(query_count(&cache, "foob", 0), 0);
    }

    #[test]
    fn cancelling_the_picker_gives_the_list_error_and_records_nothing() {
        let mut h = harness(true, vec![Answer::Pick(None)]);
        let mut cache = cache();
        let err = resolve_target(&mut h.app, &mut cache, "foob", false).expect_err("cancelled");
        assert_eq!(
            err.list_items(),
            ["github.com/acme/foobar", "github.com/acme/foobaz"]
        );
        assert_matches!(err, Error::Resolve(ResolveError::Ambiguous { .. }));
        assert!(cache.query_frecency.is_empty());
    }

    #[test]
    fn ctrl_c_in_the_picker_is_interrupted() {
        let mut h = harness(true, vec![Answer::Interrupt]);
        let err = resolve_target(&mut h.app, &mut cache(), "foob", false).expect_err("interrupted");
        assert_matches!(err, Error::Ui(UiError::Interrupted));
    }

    #[test]
    fn dominant_history_auto_resolves_and_records_without_asking() {
        let mut h = harness(true, vec![]);
        let mut cache = cache();
        with_dominant_history(&mut cache);
        let target = resolve_target(&mut h.app, &mut cache, "foob", false).expect("auto");
        assert_eq!(target.index, 1);
        assert_eq!(h.prompts.calls(), []);
        assert_eq!(query_count(&cache, "foob", 1), 4);
    }

    #[test]
    fn dominant_history_auto_resolves_without_a_tty_too() {
        let mut h = plain();
        let mut cache = cache();
        with_dominant_history(&mut cache);
        let target = resolve_target(&mut h.app, &mut cache, "foob", false).expect("auto");
        assert_eq!(target.index, 1);
    }

    #[test]
    fn pick_with_several_results_asks_without_consulting_history() {
        let mut h = harness(true, vec![Answer::Pick(Some(0))]);
        let mut cache = cache();
        with_dominant_history(&mut cache);
        let target = resolve_target(&mut h.app, &mut cache, "foob", true).expect("pick");
        assert_eq!(target.index, 0);
        assert_eq!(h.prompts.calls().len(), 1);
        assert_eq!(query_count(&cache, "foob", 0), 1);
        assert_eq!(query_count(&cache, "foob", 1), 3);
    }

    #[test]
    fn pick_asks_even_with_one_exact_match_among_several() {
        let mut h = harness(true, vec![Answer::Pick(Some(1))]);
        let mut cache = cache();
        cache.repos.push(repo("github.com/acme/foobar-cli"));
        let target = resolve_target(&mut h.app, &mut cache, "foobar", true).expect("pick");
        assert_eq!(target.index, 3);
        assert_eq!(h.prompts.calls().len(), 1);
        assert_eq!(query_count(&cache, "foobar", 3), 1);
    }

    #[test]
    fn pick_with_a_single_result_skips_the_picker() {
        let mut h = harness(true, vec![]);
        let mut cache = cache();
        let target = resolve_target(&mut h.app, &mut cache, "api", true).expect("unique");
        assert_eq!(target.index, 2);
        assert_eq!(h.prompts.calls(), []);
        assert!(cache.query_frecency.is_empty());
    }

    #[test]
    fn pick_without_a_tty_resolves_a_single_match() {
        let mut h = plain();
        let mut cache = cache();
        let target = resolve_target(&mut h.app, &mut cache, "api", true).expect("unique");
        assert_eq!(target.index, 2);
        assert!(cache.query_frecency.is_empty());
    }

    #[test]
    fn pick_without_a_tty_and_no_match_is_not_found() {
        let mut h = plain();
        let err = resolve_target(&mut h.app, &mut cache(), "zzzz", true).expect_err("none");
        assert_matches!(err, Error::Resolve(ResolveError::NotFound { .. }));
    }

    #[test]
    fn pick_without_a_tty_is_an_error_when_several_match() {
        let mut h = plain();
        let err = resolve_target(&mut h.app, &mut cache(), "foob", true).expect_err("no tty");
        assert_eq!(err.to_string(), "--pick requires an interactive terminal");
        assert_matches!(err, Error::Ui(UiError::NotInteractive { what: "--pick" }));
    }

    #[test]
    fn ensure_cloned_creates_parents_and_clones_with_the_domain_token() {
        let git = Arc::new(FakeGit::new());
        let mut h = cloning(Arc::clone(&git));
        let mut cache = cache();
        let target = resolve_target(&mut h.app, &mut cache, "foobar", false).expect("ok");
        assert!(!target.path.parent().expect("parent").exists());
        let status = ensure_cloned(&mut h.app, &cache, &target).expect("clone");
        assert_eq!(status, CloneStatus::Cloned);
        assert!(target.path.join(".git").is_dir());
        assert_eq!(
            git.cloned_urls(),
            [(target.path.clone(), cache.repos[0].clone_url.clone())]
        );
        assert_matches!(
            git.calls().as_slice(),
            [call] if call.op == GitOp::Clone && call.auth_host == Some(github())
        );
    }

    #[test]
    fn ensure_cloned_is_idempotent() {
        let git = Arc::new(FakeGit::new());
        let mut h = cloning(Arc::clone(&git));
        let mut cache = cache();
        let target = resolve_target(&mut h.app, &mut cache, "foobar", false).expect("ok");
        assert_eq!(
            ensure_cloned(&mut h.app, &cache, &target).expect("clone"),
            CloneStatus::Cloned
        );
        assert_eq!(
            ensure_cloned(&mut h.app, &cache, &target).expect("again"),
            CloneStatus::Already
        );
        assert_eq!(git.calls().len(), 1);
    }

    #[test]
    fn ensure_cloned_does_nothing_when_git_exists() {
        let git = Arc::new(FakeGit::new());
        // No token: an attempted clone would fail.
        let mut h = harness_in(
            tempfile::tempdir().expect("tempdir"),
            ScriptedInteraction::new(false, vec![]),
            Arc::clone(&git),
            FakeGh::new(),
        );
        let mut cache = cache();
        let target = resolve_target(&mut h.app, &mut cache, "foobar", false).expect("ok");
        std::fs::create_dir_all(target.path.join(".git")).expect("mkdir");
        let status = ensure_cloned(&mut h.app, &cache, &target).expect("no-op");
        assert_eq!(status, CloneStatus::Already);
        assert_eq!(git.calls(), []);
    }

    #[test]
    fn ensure_cloned_without_a_token_clones_without_auth() {
        let git = Arc::new(FakeGit::new());
        let mut h = cloning(Arc::clone(&git));
        let mut cache = cache();
        // The token is for github.com; `api` lives on ghe.corp.com.
        let target = resolve_target(&mut h.app, &mut cache, "api", false).expect("ok");
        let status = ensure_cloned(&mut h.app, &cache, &target).expect("clone");
        assert_eq!(status, CloneStatus::Cloned);
        assert_matches!(
            git.calls().as_slice(),
            [call] if call.op == GitOp::Clone && call.auth_host.is_none()
        );
    }

    #[test]
    fn ensure_cloned_with_an_invalid_env_token_fails_before_git() {
        let git = Arc::new(FakeGit::new());
        let mut h = cloning(Arc::clone(&git));
        h.app.env.github_token = EnvToken::Invalid;
        let mut cache = cache();
        let target = resolve_target(&mut h.app, &mut cache, "foobar", false).expect("ok");
        let err = ensure_cloned(&mut h.app, &cache, &target).expect_err("invalid token");
        assert_matches!(err, Error::Auth(AuthError::InvalidEnvToken));
        assert_eq!(git.calls(), []);
    }

    #[test]
    fn ensure_cloned_reports_the_git_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("code/acme/foobar");
        let failure = GitError::Failed {
            action: "clone",
            detail: "boom".to_owned(),
        };
        let script = RepoScript {
            facts: RepoFacts::Branch {
                dirty: false,
                upstream: Upstream::None,
            },
            fail: Some((GitOp::Clone, failure.clone())),
        };
        let git = Arc::new(FakeGit::new().with_repo(&dest, script));
        let mut h = harness_in(
            dir,
            ScriptedInteraction::new(false, vec![]),
            git,
            FakeGh::new().with_token(github(), "tok-123"),
        );
        let mut cache = cache();
        let target = resolve_target(&mut h.app, &mut cache, "foobar", false).expect("ok");
        assert_eq!(target.path, dest);
        let err = ensure_cloned(&mut h.app, &cache, &target).expect_err("git failed");
        assert_matches!(err, Error::Git(ref e) if *e == failure);
        assert!(!dest.join(".git").exists());
        let org_dir = dest.parent().expect("org dir");
        assert!(!org_dir.exists(), "the org dir it created is removed");
    }

    #[test]
    fn missing_dirs_lists_only_what_does_not_exist_deepest_first() {
        let dir = tempfile::tempdir().expect("tempdir");
        let deep = dir.path().join("a/b/c");
        assert_eq!(
            missing_dirs(&deep),
            [deep.clone(), dir.path().join("a/b"), dir.path().join("a")]
        );
        std::fs::create_dir_all(dir.path().join("a")).expect("mkdir");
        assert_eq!(missing_dirs(&deep), [deep.clone(), dir.path().join("a/b")]);
        assert_eq!(missing_dirs(dir.path()), Vec::<PathBuf>::new());
    }

    #[test]
    fn ensure_cloned_flushes_pending_stdout_first() {
        let mut h = cloning(Arc::new(FakeGit::new()));
        let mut cache = cache();
        let target = resolve_target(&mut h.app, &mut cache, "foobar", false).expect("ok");
        h.app.ui.stdout_line("before").expect("write");
        assert_eq!(h.out.contents(), "");
        ensure_cloned(&mut h.app, &cache, &target).expect("clone");
        assert_eq!(*h.out_at_clone.lock().expect("lock"), ["before\n"]);
    }

    #[test]
    fn record_access_records_frecency_and_saves() {
        let mut h = plain();
        let mut cache = cache();
        let target = resolve_target(&mut h.app, &mut cache, "foobar", false).expect("ok");
        record_access(&h.app, &mut cache, &target).expect("save");
        assert_eq!(
            cache
                .frecency
                .get(&cache.repos[0].slug)
                .map(|d| d.access_count),
            Some(1)
        );
        let saved = store::load(h.app.config.cache_dir());
        assert_matches!(saved, store::LoadOutcome::Loaded(ref c) if *c == cache);
    }

    #[test]
    fn record_access_keeps_what_another_process_saved_meanwhile() {
        let mut h = harness(true, vec![Answer::Pick(Some(1))]);
        let dir = h.app.config.cache_dir().to_path_buf();
        let mut cache = cache();
        store::save(&dir, &cache).expect("seed");
        let target = resolve_target(&mut h.app, &mut cache, "foob", false).expect("pick");
        assert_eq!(target.picked_for, Some(key("foob")));
        // Another `work` process records an access and saves first.
        let other = cache.repos[2].slug.clone();
        store::update(&dir, |latest| {
            let mut latest = latest.expect("seeded");
            latest.frecency.record(&other, now());
            latest
        })
        .expect("other writer");
        record_access(&h.app, &mut cache, &target).expect("save");
        let store::LoadOutcome::Loaded(saved) = store::load(&dir) else {
            panic!("cache should load");
        };
        let count = |i: usize| {
            saved
                .frecency
                .get(&saved.repos[i].slug)
                .map(|d| d.access_count)
        };
        assert_eq!((count(1), count(2)), (Some(1), Some(1)));
        assert_eq!(query_count(&saved, "foob", 1), 1);
    }

    #[test]
    fn record_access_skips_a_repo_the_newest_cache_dropped() {
        let mut h = plain();
        let dir = h.app.config.cache_dir().to_path_buf();
        let mut cache = cache();
        let target = resolve_target(&mut h.app, &mut cache, "foobar", false).expect("ok");
        assert_eq!(target.picked_for, None);
        // A refresh elsewhere removed every repo.
        store::save(&dir, &Cache::new(vec![], now())).expect("refresh");
        record_access(&h.app, &mut cache, &target).expect("save");
        assert_matches!(
            store::load(&dir),
            store::LoadOutcome::Loaded(c) if c.repos.is_empty() && c.frecency.is_empty()
        );
    }

    #[test]
    fn short_name_drops_only_the_default_domain() {
        let slug = repo("github.com/acme/foo").slug;
        assert_eq!(short_name(&slug, Some(&github())), "acme/foo");
        assert_eq!(short_name(&slug, None), "github.com/acme/foo");
    }

    #[test]
    fn is_cloned_needs_a_git_dir_or_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("acme/foo");
        assert!(!is_cloned(&repo));
        std::fs::create_dir_all(&repo).expect("mkdir");
        assert!(!is_cloned(&repo));
        std::fs::write(repo.join(".git"), "gitdir: elsewhere\n").expect("write");
        assert!(is_cloned(&repo));
        std::fs::remove_file(repo.join(".git")).expect("rm");
        std::fs::create_dir(repo.join(".git")).expect("mkdir .git");
        assert!(is_cloned(&repo));
    }
}
