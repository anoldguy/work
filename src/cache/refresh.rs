//! The freshness policy: load, maybe fetch, merge, save.

use std::time::Duration;

use crate::app::App;
use crate::auth::{AuthError, EnvToken, Tokens};
use crate::cache::fetch::{FetchPlan, fetch_all};
use crate::cache::merge::{all_failed, merge};
use crate::cache::store::{self, LoadOutcome};
use crate::cache::{Cache, CacheError};
use crate::domain::{Domain, StoredTime};
use crate::text::plural;

/// Shown by the progress display until the fetch sends its header.
const PROGRESS_TITLE: &str = "Updating repositories";

/// How hard [`load_or_refresh`] tries to get fresh data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Navigation, `list`, `favorite`: refresh when missing, corrupt, or
    /// stale. If every domain fails and a stale cache exists, warn and use
    /// it.
    AutoUpdate,
    /// `pull`: load only; a missing or corrupt cache is
    /// [`CacheError::Missing`].
    NoUpdate,
    /// `update`: always refresh; on total failure, error and keep the
    /// existing file.
    Force {
        /// `--all-orgs`: ignore favourites.
        all_orgs: bool,
    },
}

/// Returns an owned cache, held by the command separately from `app` so both
/// can be borrowed mutably. Fetching runs under `app.ui.with_progress`.
///
/// The fetch runs without holding the cache lock; the merge and save then
/// run inside [`store::update`], so the merge starts from the newest copy on
/// disk and frecency recorded by another `work` process meanwhile is kept.
/// A corrupt file is renamed to `cache.json.corrupt` only once a refresh has
/// succeeded, immediately before the new cache is written, so a failed
/// refresh never touches the disk. An invalid `GITHUB_TOKEN` fails a needed
/// refresh outright instead of falling back to a stale cache.
///
/// # Errors
///
/// [`CacheError::Missing`] under `NoUpdate`; [`CacheError::Auth`] when a
/// refresh is needed and `GITHUB_TOKEN` is invalid; [`CacheError::AllFailed`]
/// when a needed refresh fails for every domain and there is nothing to fall
/// back on (or under `Force`); store errors.
pub fn load_or_refresh(app: &mut App, freshness: Freshness) -> Result<Cache, CacheError> {
    let previous = match store::load(app.config.cache_dir()) {
        LoadOutcome::Loaded(cache) => Some(cache),
        LoadOutcome::Missing => None,
        LoadOutcome::Corrupt(reason) => {
            app.ui
                .warn(&format!("the repository cache is unreadable ({reason})"));
            None
        }
    };

    match (freshness, previous) {
        (Freshness::NoUpdate, previous) => previous.ok_or(CacheError::Missing),
        (Freshness::AutoUpdate, Some(cache)) if !cache.is_stale(app.now) => Ok(cache),
        (_, previous) => refresh(app, freshness, previous),
    }
}

/// Fetches every effective domain, then merges and saves, or falls back.
fn refresh(
    app: &mut App,
    freshness: Freshness,
    previous: Option<Cache>,
) -> Result<Cache, CacheError> {
    if matches!(app.env.github_token, EnvToken::Invalid) {
        return Err(AuthError::InvalidEnvToken.into());
    }

    let all_orgs = matches!(freshness, Freshness::Force { all_orgs: true });
    let plan = FetchPlan::from_config(&app.config, all_orgs);
    let domains: Vec<Domain> = plan.domains.iter().map(|d| d.domain.clone()).collect();
    let tokens = Tokens::resolve(&*app.gh, &app.env.github_token, &domains);
    let api = &*app.api;
    let outcomes = app
        .ui
        .with_progress(PROGRESS_TITLE, |sink| fetch_all(&plan, api, &tokens, sink));

    if all_failed(&outcomes) {
        return match (freshness, previous) {
            (Freshness::AutoUpdate, Some(stale)) => {
                app.ui.warn(&format!(
                    "could not refresh the repository cache; using the copy from {}",
                    describe_age(stale.updated_at, app.now)
                ));
                Ok(stale)
            }
            _ => Err(CacheError::AllFailed),
        };
    }

    let now = app.now;
    let filters = app.config.filters();
    store::update(app.config.cache_dir(), |latest| {
        // The newest copy wins; if it vanished or broke meanwhile, fall back
        // to what was read before the fetch so nothing is dropped.
        merge(latest.or(previous), outcomes, &domains, filters, now)
    })
}

/// `3 days ago`, `5 hours ago`, `12 minutes ago`, or `a future date`.
fn describe_age(updated_at: StoredTime, now: StoredTime) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    if updated_at.age_at(now) == Duration::ZERO && now.age_at(updated_at) > Duration::ZERO {
        return "a future date".to_owned();
    }
    let secs = updated_at.age_at(now).as_secs();
    let (n, one, many) = match secs {
        s if s >= DAY => (s / DAY, "day", "days"),
        s if s >= HOUR => (s / HOUR, "hour", "hours"),
        s => (s / MINUTE, "minute", "minutes"),
    };
    format!("{n} {} ago", plural(n, one, many))
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;

    use super::*;
    use crate::app::Adapters;
    use crate::auth::SecretToken;
    use crate::cache::store::cache_file;
    use crate::config::{self, Locations};
    use crate::domain::{Domain, Org, Repo, StoredTime};
    use crate::env::EnvSnapshot;
    use crate::gh::{Gh, GhError};
    use crate::github::{ApiError, GitHubApi, Listing, Page};
    use crate::testing::{
        ApiCall, Endpoint, FakeGh, FakeGit, FakeGitHub, NullGitHub, ScriptedInteraction, SharedBuf,
    };
    use crate::ui::{Mode, Ui};

    const NOW: i64 = 1_700_000_000;
    const DAY: i64 = 86_400;

    fn t(secs: i64) -> StoredTime {
        StoredTime::from_unix_seconds(secs).expect("time")
    }

    fn dom(s: &str) -> Domain {
        s.parse().expect("domain")
    }

    fn org(s: &str) -> Org {
        s.parse().expect("org")
    }

    fn repo(owner: &str, name: &str) -> Repo {
        serde_json::from_value(serde_json::json!({
            "slug": format!("github.com/{owner}/{name}"),
            "clone_url": format!("https://github.com/{owner}/{name}.git"),
            "html_url": format!("https://github.com/{owner}/{name}"),
        }))
        .expect("repo")
    }

    fn slugs(cache: &Cache) -> Vec<String> {
        let mut s: Vec<String> = cache.repos.iter().map(|r| r.slug.to_string()).collect();
        s.sort();
        s
    }

    /// Lets a test keep a handle on the `FakeGitHub` it gives the `App`.
    #[derive(Debug)]
    struct SharedApi(Arc<FakeGitHub>);

    impl GitHubApi for SharedApi {
        fn viewer(&self, d: &Domain, t: &SecretToken) -> Result<Org, ApiError> {
            self.0.viewer(d, t)
        }
        fn member_orgs(&self, d: &Domain, t: &SecretToken) -> Result<Vec<Org>, ApiError> {
            self.0.member_orgs(d, t)
        }
        fn user_repos(
            &self,
            d: &Domain,
            t: &SecretToken,
            on_page: &mut dyn FnMut(Page),
        ) -> Result<Listing, ApiError> {
            self.0.user_repos(d, t, on_page)
        }
        fn org_repos(
            &self,
            d: &Domain,
            t: &SecretToken,
            org: &Org,
            on_page: &mut dyn FnMut(Page),
        ) -> Result<Listing, ApiError> {
            self.0.org_repos(d, t, org, on_page)
        }
    }

    /// Lets a test keep a handle on the `FakeGh` it gives the `App`.
    #[derive(Debug)]
    struct SharedGh(Arc<FakeGh>);

    impl Gh for SharedGh {
        fn auth_token(&self, d: &Domain) -> Result<Option<SecretToken>, GhError> {
            self.0.auth_token(d)
        }
        fn login(&self, d: &Domain) -> Result<(), GhError> {
            self.0.login(d)
        }
        fn logout(&self, d: &Domain) -> Result<(), GhError> {
            self.0.logout(d)
        }
    }

    /// `github.com` as `me`, member of `acme` and `widgets`.
    fn healthy_api() -> FakeGitHub {
        FakeGitHub::new()
            .with_viewer(dom("github.com"), org("me"))
            .with_member_orgs(dom("github.com"), vec![org("acme"), org("widgets")])
            .with_user_repos(dom("github.com"), vec![vec![repo("me", "dotfiles")]])
            .with_org_repos(
                dom("github.com"),
                org("acme"),
                vec![vec![repo("acme", "api")]],
            )
            .with_org_repos(
                dom("github.com"),
                org("widgets"),
                vec![vec![repo("widgets", "gear")]],
            )
    }

    fn shared(api: FakeGitHub) -> (Arc<FakeGitHub>, Box<dyn GitHubApi>) {
        let api = Arc::new(api);
        (Arc::clone(&api), Box::new(SharedApi(api)))
    }

    struct Setup {
        api: Box<dyn GitHubApi>,
        token: EnvToken,
        gh: FakeGh,
        config: Option<&'static str>,
        mode: Mode,
    }

    impl Setup {
        fn new(api: Box<dyn GitHubApi>) -> Self {
            Self {
                api,
                token: EnvToken::Valid(SecretToken::new("ghp_test").expect("token")),
                gh: FakeGh::new(),
                config: None,
                mode: Mode::Plain,
            }
        }

        fn build(self) -> Harness {
            let dir = tempfile::tempdir().expect("tempdir");
            let env = EnvSnapshot {
                home: Some(dir.path().to_path_buf()),
                github_token: self.token,
                ..EnvSnapshot::default()
            };
            let locations = Locations::from_env(&env).expect("locations");
            if let Some(text) = self.config {
                fs::create_dir_all(locations.config_file.parent().expect("parent")).expect("mkdir");
                fs::write(&locations.config_file, text).expect("config");
            }
            let config = config::store::load(&locations).expect("config");
            let err = SharedBuf::new();
            let ui = Ui::for_test_mode(
                self.mode,
                Box::new(SharedBuf::new()),
                Box::new(err.clone()),
                Box::new(ScriptedInteraction::new(false, vec![])),
            );
            let gh = Arc::new(self.gh);
            let adapters = Adapters {
                git: Box::new(FakeGit::new()),
                gh: Box::new(SharedGh(Arc::clone(&gh))),
                api: self.api,
            };
            let app = App::with_parts(env, locations, config, ui, adapters, t(NOW));
            Harness {
                _dir: dir,
                app,
                err,
                gh,
            }
        }
    }

    struct Harness {
        _dir: tempfile::TempDir,
        app: App,
        err: SharedBuf,
        gh: Arc<FakeGh>,
    }

    impl Harness {
        fn cache_path(&self) -> PathBuf {
            cache_file(self.app.config.cache_dir())
        }

        fn corrupt_path(&self) -> PathBuf {
            self.cache_path().with_file_name("cache.json.corrupt")
        }

        fn seed(&self, cache: &Cache) {
            store::save(self.app.config.cache_dir(), cache).expect("seed");
        }

        fn seed_raw(&self, text: &str) {
            let path = self.cache_path();
            fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            fs::write(path, text).expect("seed");
        }

        fn on_disk(&self) -> Vec<u8> {
            fs::read(self.cache_path()).expect("cache file")
        }
    }

    fn old_cache(updated_at: i64) -> Cache {
        Cache::new(vec![repo("acme", "old")], t(updated_at))
    }

    fn org_calls(calls: &[ApiCall]) -> Vec<Org> {
        let mut orgs: Vec<Org> = calls
            .iter()
            .filter_map(|c| match &c.endpoint {
                Endpoint::OrgRepos(o) => Some(o.clone()),
                _ => None,
            })
            .collect();
        orgs.sort();
        orgs
    }

    // ---- AutoUpdate --------------------------------------------------------

    #[test]
    fn auto_update_returns_a_fresh_cache_without_fetching() {
        let mut h = Setup::new(Box::new(NullGitHub)).build();
        let fresh = old_cache(NOW - DAY);
        h.seed(&fresh);
        let cache = load_or_refresh(&mut h.app, Freshness::AutoUpdate).expect("cache");
        assert_eq!(cache, fresh);
        assert_eq!(h.err.contents(), "");
    }

    #[test]
    fn auto_update_refreshes_a_stale_cache_and_saves_it() {
        let (api, boxed) = shared(healthy_api());
        let mut h = Setup::new(boxed).build();
        h.seed(&old_cache(NOW - DAY - 1));
        let cache = load_or_refresh(&mut h.app, Freshness::AutoUpdate).expect("cache");
        assert_eq!(
            slugs(&cache),
            [
                "github.com/acme/api",
                "github.com/me/dotfiles",
                "github.com/widgets/gear"
            ]
        );
        assert_eq!(cache.updated_at, t(NOW));
        assert_eq!(
            api.calls().first().map(|c| &c.endpoint),
            Some(&Endpoint::Viewer)
        );
        assert_matches!(
            store::load(h.app.config.cache_dir()),
            LoadOutcome::Loaded(saved) if saved == cache
        );
    }

    #[test]
    fn auto_update_refreshes_a_cache_dated_beyond_the_clock_skew() {
        let (api, boxed) = shared(healthy_api());
        let mut h = Setup::new(boxed).build();
        h.seed(&old_cache(NOW + 301));
        let cache = load_or_refresh(&mut h.app, Freshness::AutoUpdate).expect("cache");
        assert_eq!(cache.updated_at, t(NOW));
        assert_eq!(
            api.calls().first().map(|c| &c.endpoint),
            Some(&Endpoint::Viewer)
        );
    }

    #[test]
    fn auto_update_builds_a_missing_cache() {
        let (_, boxed) = shared(healthy_api());
        let mut h = Setup::new(boxed).build();
        let cache = load_or_refresh(&mut h.app, Freshness::AutoUpdate).expect("cache");
        assert_eq!(cache.repos.len(), 3);
        assert_matches!(
            store::load(h.app.config.cache_dir()),
            LoadOutcome::Loaded(saved) if saved == cache
        );
    }

    #[test]
    fn auto_update_quarantines_a_corrupt_cache_then_rebuilds() {
        let (_, boxed) = shared(healthy_api());
        let mut h = Setup::new(boxed).build();
        h.seed_raw("{not json");
        let cache = load_or_refresh(&mut h.app, Freshness::AutoUpdate).expect("cache");
        assert_eq!(cache.repos.len(), 3);
        assert_eq!(
            fs::read_to_string(h.corrupt_path()).expect("quarantined"),
            "{not json"
        );
        assert_matches!(
            store::load(h.app.config.cache_dir()),
            LoadOutcome::Loaded(saved) if saved == cache
        );
        let err = h.err.contents();
        assert!(
            err.contains("warning: the repository cache is unreadable"),
            "{err}"
        );
    }

    #[test]
    fn auto_update_uses_a_stale_cache_with_a_warning_when_every_domain_fails() {
        let mut h = Setup::new(Box::new(FakeGitHub::new())).build();
        let stale = old_cache(NOW - 2 * DAY);
        h.seed(&stale);
        let before = h.on_disk();
        let cache = load_or_refresh(&mut h.app, Freshness::AutoUpdate).expect("stale");
        assert_eq!(cache, stale);
        assert_eq!(h.on_disk(), before);
        let err = h.err.contents();
        assert!(err.contains("cannot refresh github.com"), "{err}");
        assert!(err.contains("using the copy from 2 days ago"), "{err}");
        assert!(err.lines().all(|l| l.starts_with("warning: ")), "{err}");
    }

    #[test]
    fn auto_update_errors_when_every_domain_fails_and_there_is_no_cache() {
        let mut h = Setup::new(Box::new(FakeGitHub::new())).build();
        assert_matches!(
            load_or_refresh(&mut h.app, Freshness::AutoUpdate),
            Err(CacheError::AllFailed)
        );
        assert!(!h.cache_path().exists());
    }

    #[test]
    fn a_corrupt_file_stays_in_place_when_the_rebuild_fails() {
        let mut h = Setup::new(Box::new(FakeGitHub::new())).build();
        h.seed_raw("{not json");
        assert_matches!(
            load_or_refresh(&mut h.app, Freshness::AutoUpdate),
            Err(CacheError::AllFailed)
        );
        assert_eq!(h.on_disk(), b"{not json");
        assert!(!h.corrupt_path().exists());
    }

    #[test]
    fn a_failed_org_warns_and_carries_its_repos_forward() {
        let api =
            healthy_api().with_status(dom("github.com"), Endpoint::OrgRepos(org("acme")), 502);
        let (_, boxed) = shared(api);
        let mut h = Setup::new(boxed).build();
        h.seed(&old_cache(NOW - 2 * DAY));
        let cache = load_or_refresh(&mut h.app, Freshness::AutoUpdate).expect("cache");
        assert!(slugs(&cache).contains(&"github.com/acme/old".to_owned()));
        assert_eq!(cache.updated_at, t(NOW));
        let err = h.err.contents();
        assert!(err.starts_with("warning: ") && err.contains("502"), "{err}");
    }

    #[test]
    fn warnings_follow_the_progress_display_in_tty_mode() {
        let api =
            healthy_api().with_status(dom("github.com"), Endpoint::OrgRepos(org("acme")), 502);
        let (_, boxed) = shared(api);
        let mut setup = Setup::new(boxed);
        setup.mode = Mode::Tty;
        let mut h = setup.build();
        load_or_refresh(&mut h.app, Freshness::AutoUpdate).expect("cache");
        let err = h.err.contents();
        let last = err.lines().last().expect("output");
        assert!(
            last.starts_with("warning: ") && last.contains("502"),
            "{err}"
        );
        assert_eq!(err.matches("warning:").count(), 1, "{err}");
    }

    // ---- NoUpdate ----------------------------------------------------------

    #[test]
    fn no_update_returns_a_stale_cache_without_fetching() {
        let mut h = Setup::new(Box::new(NullGitHub)).build();
        let stale = old_cache(NOW - 10 * DAY);
        h.seed(&stale);
        let cache = load_or_refresh(&mut h.app, Freshness::NoUpdate).expect("cache");
        assert_eq!(cache, stale);
    }

    #[test]
    fn no_update_reports_a_missing_cache() {
        let mut h = Setup::new(Box::new(NullGitHub)).build();
        assert_matches!(
            load_or_refresh(&mut h.app, Freshness::NoUpdate),
            Err(CacheError::Missing)
        );
        assert!(!h.cache_path().exists());
    }

    #[test]
    fn no_update_treats_a_corrupt_cache_as_missing_and_leaves_it() {
        let mut h = Setup::new(Box::new(NullGitHub)).build();
        h.seed_raw("[]");
        assert_matches!(
            load_or_refresh(&mut h.app, Freshness::NoUpdate),
            Err(CacheError::Missing)
        );
        assert_eq!(h.on_disk(), b"[]");
        let err = h.err.contents();
        assert!(
            err.contains("warning: the repository cache is unreadable"),
            "{err}"
        );
        assert_eq!(
            CacheError::Missing.to_string(),
            "no usable repository cache (run 'work update')"
        );
    }

    // ---- Force -------------------------------------------------------------

    #[test]
    fn force_refreshes_even_a_fresh_cache() {
        let (api, boxed) = shared(healthy_api());
        let mut h = Setup::new(boxed).build();
        h.seed(&old_cache(NOW - 60));
        let cache =
            load_or_refresh(&mut h.app, Freshness::Force { all_orgs: false }).expect("cache");
        assert_eq!(cache.updated_at, t(NOW));
        assert!(!slugs(&cache).contains(&"github.com/acme/old".to_owned()));
        assert_eq!(
            api.calls().first().map(|c| &c.endpoint),
            Some(&Endpoint::Viewer)
        );
    }

    #[test]
    fn force_errors_and_keeps_the_existing_file_when_every_domain_fails() {
        let mut h = Setup::new(Box::new(FakeGitHub::new())).build();
        h.seed(&old_cache(NOW - 60));
        let before = h.on_disk();
        assert_matches!(
            load_or_refresh(&mut h.app, Freshness::Force { all_orgs: false }),
            Err(CacheError::AllFailed)
        );
        assert_eq!(h.on_disk(), before);
    }

    const FAVORITES: &str = "[domains.\"github.com\"]\nfavorite_orgs = [\"acme\"]\n";

    #[test]
    fn force_honours_favourites() {
        let (api, boxed) = shared(healthy_api());
        let mut setup = Setup::new(boxed);
        setup.config = Some(FAVORITES);
        let mut h = setup.build();
        load_or_refresh(&mut h.app, Freshness::Force { all_orgs: false }).expect("cache");
        assert_eq!(org_calls(&api.calls()), [org("acme")]);
    }

    #[test]
    fn force_all_orgs_bypasses_favourites() {
        let (api, boxed) = shared(healthy_api());
        let mut setup = Setup::new(boxed);
        setup.config = Some(FAVORITES);
        let mut h = setup.build();
        load_or_refresh(&mut h.app, Freshness::Force { all_orgs: true }).expect("cache");
        assert_eq!(org_calls(&api.calls()), [org("acme"), org("widgets")]);
    }

    // ---- Tokens ------------------------------------------------------------

    #[test]
    fn an_invalid_env_token_is_a_hard_auth_error_in_every_refreshing_mode() {
        for freshness in [Freshness::AutoUpdate, Freshness::Force { all_orgs: false }] {
            let mut setup = Setup::new(Box::new(NullGitHub));
            setup.token = EnvToken::Invalid;
            let mut h = setup.build();
            h.seed(&old_cache(NOW - 2 * DAY));
            let before = h.on_disk();
            assert_matches!(
                load_or_refresh(&mut h.app, freshness),
                Err(CacheError::Auth(AuthError::InvalidEnvToken))
            );
            assert_eq!(h.on_disk(), before);
            assert_eq!(h.err.contents(), "");
            assert_eq!(h.gh.calls().len(), 0);
        }
    }

    #[test]
    fn an_invalid_env_token_is_ignored_when_no_refresh_is_needed() {
        let mut setup = Setup::new(Box::new(NullGitHub));
        setup.token = EnvToken::Invalid;
        let mut h = setup.build();
        h.seed(&old_cache(NOW));
        assert_matches!(load_or_refresh(&mut h.app, Freshness::AutoUpdate), Ok(_));
    }

    #[test]
    fn gh_tokens_are_resolved_once_per_domain_per_refresh() {
        let (_, boxed) = shared(healthy_api());
        let mut setup = Setup::new(boxed);
        setup.token = EnvToken::Unset;
        setup.gh = FakeGh::new().with_token(dom("github.com"), "gho_test");
        setup.config = Some(TWO_DOMAINS);
        let mut h = setup.build();
        load_or_refresh(&mut h.app, Freshness::AutoUpdate).expect("cache");
        let mut hosts: Vec<Domain> = h.gh.calls().into_iter().map(|c| c.host).collect();
        hosts.sort();
        assert_eq!(hosts, [dom("ghe.example.com"), dom("github.com")]);
    }

    // ---- Multiple domains, frecency, concurrency ---------------------------

    const TWO_DOMAINS: &str =
        "[domains.\"github.com\"]\nis_default = true\n\n[domains.\"ghe.example.com\"]\n";

    fn ghe_repo() -> Repo {
        serde_json::from_value(serde_json::json!({
            "slug": "ghe.example.com/corp/infra",
            "clone_url": "https://ghe.example.com/corp/infra.git",
            "html_url": "https://ghe.example.com/corp/infra",
        }))
        .expect("repo")
    }

    #[test]
    fn a_failed_domain_is_carried_forward_while_the_others_refresh() {
        let (_, boxed) = shared(healthy_api());
        let mut setup = Setup::new(boxed);
        setup.token = EnvToken::Unset;
        setup.gh = FakeGh::new().with_token(dom("github.com"), "gho_test");
        setup.config = Some(TWO_DOMAINS);
        let mut h = setup.build();
        h.seed(&Cache::new(
            vec![repo("acme", "old"), ghe_repo()],
            t(NOW - 2 * DAY),
        ));
        let cache = load_or_refresh(&mut h.app, Freshness::AutoUpdate).expect("cache");
        assert_eq!(cache.updated_at, t(NOW));
        assert_eq!(
            slugs(&cache),
            [
                "ghe.example.com/corp/infra",
                "github.com/acme/api",
                "github.com/me/dotfiles",
                "github.com/widgets/gear"
            ]
        );
        let err = h.err.contents();
        assert_eq!(
            err.matches("warning: cannot refresh ghe.example.com")
                .count(),
            1,
            "{err}"
        );
        assert_matches!(
            store::load(h.app.config.cache_dir()),
            LoadOutcome::Loaded(saved) if saved == cache
        );
    }

    #[test]
    fn refresh_keeps_frecency_for_live_repos_and_drops_it_for_gone_ones() {
        let (_, boxed) = shared(healthy_api());
        let mut h = Setup::new(boxed).build();
        let mut seeded = Cache::new(
            vec![repo("acme", "api"), repo("acme", "old")],
            t(NOW - 2 * DAY),
        );
        seeded
            .frecency
            .record(&repo("acme", "api").slug, t(NOW - DAY));
        seeded
            .frecency
            .record(&repo("acme", "old").slug, t(NOW - DAY));
        h.seed(&seeded);
        let cache = load_or_refresh(&mut h.app, Freshness::AutoUpdate).expect("cache");
        assert_eq!(
            cache
                .frecency
                .get(&repo("acme", "api").slug)
                .map(|d| d.access_count),
            Some(1)
        );
        assert_eq!(cache.frecency.get(&repo("acme", "old").slug), None);
    }

    /// Saves an access through [`store::update`] while the fetch is running,
    /// as `work cd` finishing in another shell would.
    #[derive(Debug)]
    struct WriterDuringFetch {
        inner: FakeGitHub,
        dir: PathBuf,
    }

    impl GitHubApi for WriterDuringFetch {
        fn viewer(&self, d: &Domain, t: &SecretToken) -> Result<Org, ApiError> {
            self.inner.viewer(d, t)
        }
        fn member_orgs(&self, d: &Domain, t: &SecretToken) -> Result<Vec<Org>, ApiError> {
            self.inner.member_orgs(d, t)
        }
        fn user_repos(
            &self,
            d: &Domain,
            tok: &SecretToken,
            on_page: &mut dyn FnMut(Page),
        ) -> Result<Listing, ApiError> {
            store::update(&self.dir, |latest| {
                let mut latest = latest.expect("seeded");
                latest.frecency.record(&repo("acme", "api").slug, t(NOW));
                latest
            })
            .expect("concurrent save");
            self.inner.user_repos(d, tok, on_page)
        }
        fn org_repos(
            &self,
            d: &Domain,
            tok: &SecretToken,
            org: &Org,
            on_page: &mut dyn FnMut(Page),
        ) -> Result<Listing, ApiError> {
            self.inner.org_repos(d, tok, org, on_page)
        }
    }

    #[test]
    fn an_access_saved_during_the_fetch_survives_the_refresh() {
        // Build once to learn the cache dir, then swap in the writer.
        let mut h = Setup::new(Box::new(NullGitHub)).build();
        let dir = h.app.config.cache_dir().to_path_buf();
        h.app.api = Box::new(WriterDuringFetch {
            inner: healthy_api(),
            dir,
        });
        h.seed(&Cache::new(vec![repo("acme", "api")], t(NOW - 2 * DAY)));
        let cache =
            load_or_refresh(&mut h.app, Freshness::Force { all_orgs: false }).expect("cache");
        let api = repo("acme", "api").slug;
        assert_eq!(cache.frecency.get(&api).map(|d| d.access_count), Some(1));
        assert_matches!(
            store::load(h.app.config.cache_dir()),
            LoadOutcome::Loaded(saved) if saved == cache
        );
    }

    // ---- Corrupt and missing under Force ----------------------------------

    #[test]
    fn force_quarantines_a_corrupt_cache_then_rebuilds() {
        let (_, boxed) = shared(healthy_api());
        let mut h = Setup::new(boxed).build();
        h.seed_raw("{not json");
        let cache =
            load_or_refresh(&mut h.app, Freshness::Force { all_orgs: false }).expect("cache");
        assert_eq!(cache.repos.len(), 3);
        assert_eq!(
            fs::read_to_string(h.corrupt_path()).expect("quarantined"),
            "{not json"
        );
    }

    #[test]
    fn force_with_no_cache_errors_and_writes_nothing_when_every_domain_fails() {
        let mut h = Setup::new(Box::new(FakeGitHub::new())).build();
        assert_matches!(
            load_or_refresh(&mut h.app, Freshness::Force { all_orgs: false }),
            Err(CacheError::AllFailed)
        );
        assert!(!h.cache_path().exists());
    }

    #[test]
    fn a_cache_path_that_is_a_directory_is_quarantined_over_an_old_corrupt_file() {
        let (_, boxed) = shared(healthy_api());
        let mut h = Setup::new(boxed).build();
        fs::create_dir_all(h.cache_path()).expect("dir");
        fs::write(h.corrupt_path(), "older").expect("old corrupt");
        let cache = load_or_refresh(&mut h.app, Freshness::AutoUpdate).expect("cache");
        assert!(h.corrupt_path().is_dir());
        assert_matches!(
            store::load(h.app.config.cache_dir()),
            LoadOutcome::Loaded(saved) if saved == cache
        );
    }

    #[test]
    fn ages_are_described_in_the_largest_whole_unit() {
        assert_eq!(describe_age(t(NOW - 2 * DAY - 5), t(NOW)), "2 days ago");
        assert_eq!(describe_age(t(NOW - DAY), t(NOW)), "1 day ago");
        assert_eq!(describe_age(t(NOW - 3 * 3600), t(NOW)), "3 hours ago");
        assert_eq!(describe_age(t(NOW - 60), t(NOW)), "1 minute ago");
        assert_eq!(describe_age(t(NOW + 600), t(NOW)), "a future date");
    }
}
