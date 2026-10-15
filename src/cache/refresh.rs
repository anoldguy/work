//! The freshness policy: load, maybe fetch, merge, save.

use std::time::Duration;

use crate::app::App;
use crate::cache::fetch::{FetchPlan, fetch_all};
use crate::cache::merge::{all_failed, merge};
use crate::cache::store::{self, LoadOutcome};
use crate::cache::{Cache, CacheError};
use crate::domain::{Domain, StoredTime};
use crate::text::plural;
use crate::ui::Ui;

/// Shown by the progress display until the fetch sends its header.
const PROGRESS_TITLE: &str = "Updating repositories";

/// How hard [`load_or_refresh`] tries to get fresh data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Navigation, `list`, `favorite`: refresh when missing, corrupt, or
    /// stale. If every domain fails and a stale cache exists, warn and use
    /// it.
    AutoUpdate,
    /// `pull` and `sync`: load only; a missing or corrupt cache is
    /// [`CacheError::Missing`].
    NoUpdate,
    /// `update`: always refresh; on total failure, error and keep the
    /// existing file.
    Force {
        /// `--all-orgs`: ignore favourites.
        all_orgs: bool,
    },
}

/// Returns an owned cache. Fetching runs under `ui.with_progress`.
///
/// The merge starts from the copy read before the fetch and is written with
/// [`store::replace`]; a concurrent refresh's copy is simply overwritten
/// (both hold a complete listing). Frecency is never read or written here.
/// A corrupt file is renamed to `cache.json.corrupt` only once a refresh has
/// succeeded, immediately before the new cache is written, so a failed
/// refresh never touches the disk.
///
/// # Errors
///
/// [`CacheError::Missing`] under `NoUpdate`; [`CacheError::AllFailed`]
/// when a needed refresh fails for every domain and there is nothing to fall
/// back on (or under `Force`); store errors.
pub fn load_or_refresh(app: &App, ui: &mut Ui, freshness: Freshness) -> Result<Cache, CacheError> {
    let previous = match store::load(app.config.cache_dir()) {
        LoadOutcome::Loaded(cache) => Some(cache),
        LoadOutcome::Missing => None,
        LoadOutcome::Corrupt(reason) => {
            ui.warn(&format!("the repository cache is unreadable ({reason})"));
            None
        }
    };

    match (freshness, previous) {
        (Freshness::NoUpdate, previous) => previous.ok_or(CacheError::Missing),
        (Freshness::AutoUpdate, Some(cache)) if !cache.is_stale(app.now) => Ok(cache),
        (_, previous) => refresh(app, ui, freshness, previous),
    }
}

/// Fetches every effective domain, then merges and saves, or falls back.
fn refresh(
    app: &App,
    ui: &mut Ui,
    freshness: Freshness,
    previous: Option<Cache>,
) -> Result<Cache, CacheError> {
    let all_orgs = matches!(freshness, Freshness::Force { all_orgs: true });
    let plan = FetchPlan::from_config(&app.config, all_orgs);
    let domains: Vec<Domain> = plan.domains.iter().map(|d| d.domain.clone()).collect();
    let gh = &*app.gh;
    let outcomes = ui.with_progress(PROGRESS_TITLE, |tasks| fetch_all(&plan, gh, tasks));

    if all_failed(&outcomes) {
        return match (freshness, previous) {
            (Freshness::AutoUpdate, Some(stale)) => {
                ui.warn(&format!(
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
    let cache = merge(previous, outcomes, &domains, filters, now);
    store::replace(app.config.cache_dir(), &cache)?;
    Ok(cache)
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
    use crate::cache::store::cache_file;
    use crate::config::{self, Locations};
    use crate::domain::{Domain, Org, Repo, StoredTime};
    use crate::env::EnvSnapshot;
    use crate::testing::{ApiCall, Endpoint, FakeGh, FakeGit, ScriptedInteraction, SharedBuf};
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

    /// `github.com` as `me`, member of `acme` and `widgets`.
    fn healthy() -> FakeGh {
        FakeGh::new()
            .with_viewer(dom("github.com"), org("me"))
            .with_member_orgs(dom("github.com"), vec![org("acme"), org("widgets")])
            .with_user_repos(dom("github.com"), vec![repo("me", "dotfiles")])
            .with_org_repos(dom("github.com"), org("acme"), vec![repo("acme", "api")])
            .with_org_repos(
                dom("github.com"),
                org("widgets"),
                vec![repo("widgets", "gear")],
            )
    }

    struct Setup {
        gh: FakeGh,
        config: Option<&'static str>,
        mode: Mode,
    }

    impl Setup {
        fn new(gh: FakeGh) -> Self {
            Self {
                gh,
                config: None,
                mode: Mode::Plain,
            }
        }

        fn build(self) -> Harness {
            let dir = tempfile::tempdir().expect("tempdir");
            let env = EnvSnapshot {
                home: Some(dir.path().to_path_buf()),
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
                gh: Box::new(Arc::clone(&gh)),
            };
            let app = App::with_parts(env, locations, config, adapters, t(NOW));
            Harness {
                _dir: dir,
                app,
                ui,
                err,
                gh,
            }
        }
    }

    struct Harness {
        _dir: tempfile::TempDir,
        app: App,
        ui: Ui,
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
        let mut h = Setup::new(FakeGh::new()).build();
        let fresh = old_cache(NOW - DAY);
        h.seed(&fresh);
        let cache = load_or_refresh(&h.app, &mut h.ui, Freshness::AutoUpdate).expect("cache");
        assert_eq!(cache, fresh);
        assert_eq!(h.err.contents(), "");
        assert_eq!(h.gh.api_calls(), []);
    }

    #[test]
    fn auto_update_refreshes_a_stale_cache_and_saves_it() {
        let mut h = Setup::new(healthy()).build();
        h.seed(&old_cache(NOW - DAY - 1));
        let cache = load_or_refresh(&h.app, &mut h.ui, Freshness::AutoUpdate).expect("cache");
        assert_eq!(
            slugs(&cache),
            [
                "github.com/acme/api",
                "github.com/me/dotfiles",
                "github.com/widgets/gear"
            ]
        );
        assert_eq!(cache.updated_at, t(NOW));
        assert_matches!(
            store::load(h.app.config.cache_dir()),
            LoadOutcome::Loaded(saved) if saved == cache
        );
    }

    #[test]
    fn auto_update_quarantines_a_corrupt_cache_then_rebuilds() {
        let mut h = Setup::new(healthy()).build();
        h.seed_raw("{not json");
        let cache = load_or_refresh(&h.app, &mut h.ui, Freshness::AutoUpdate).expect("cache");
        assert_eq!(cache.repos.len(), 3);
        assert_eq!(
            fs::read_to_string(h.corrupt_path()).expect("quarantined"),
            "{not json"
        );
        assert_matches!(
            store::load(h.app.config.cache_dir()),
            LoadOutcome::Loaded(saved) if saved == cache
        );
    }

    #[test]
    fn auto_update_uses_a_stale_cache_with_a_warning_when_every_domain_fails() {
        let mut h = Setup::new(FakeGh::new()).build();
        let stale = old_cache(NOW - 2 * DAY);
        h.seed(&stale);
        let before = h.on_disk();
        let cache = load_or_refresh(&h.app, &mut h.ui, Freshness::AutoUpdate).expect("stale");
        assert_eq!(cache, stale);
        assert_eq!(h.on_disk(), before);
        let err = h.err.contents();
        assert!(err.contains("cannot refresh github.com"), "{err}");
        assert!(err.contains("using the copy from 2 days ago"), "{err}");
        assert!(err.lines().all(|l| l.starts_with("warning: ")), "{err}");
    }

    #[test]
    fn auto_update_errors_when_every_domain_fails_and_there_is_no_cache() {
        let mut h = Setup::new(FakeGh::new()).build();
        assert_matches!(
            load_or_refresh(&h.app, &mut h.ui, Freshness::AutoUpdate),
            Err(CacheError::AllFailed)
        );
        assert!(!h.cache_path().exists());
    }

    #[test]
    fn a_corrupt_file_stays_in_place_when_the_rebuild_fails() {
        let mut h = Setup::new(FakeGh::new()).build();
        h.seed_raw("{not json");
        assert_matches!(
            load_or_refresh(&h.app, &mut h.ui, Freshness::AutoUpdate),
            Err(CacheError::AllFailed)
        );
        assert_eq!(h.on_disk(), b"{not json");
        assert!(!h.corrupt_path().exists());
    }

    #[test]
    fn a_failed_org_warns_and_carries_its_repos_forward() {
        let gh = healthy().with_failure(
            dom("github.com"),
            Endpoint::OrgRepos(org("acme")),
            "gh: Server Error (HTTP 502)",
        );
        let mut setup = Setup::new(gh);
        setup.mode = Mode::Tty;
        let mut h = setup.build();
        h.seed(&old_cache(NOW - 2 * DAY));
        let cache = load_or_refresh(&h.app, &mut h.ui, Freshness::AutoUpdate).expect("cache");
        assert!(slugs(&cache).contains(&"github.com/acme/old".to_owned()));
        assert_eq!(cache.updated_at, t(NOW));
        // The one warning follows the progress display.
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
        let mut h = Setup::new(FakeGh::new()).build();
        let stale = old_cache(NOW - 10 * DAY);
        h.seed(&stale);
        let cache = load_or_refresh(&h.app, &mut h.ui, Freshness::NoUpdate).expect("cache");
        assert_eq!(cache, stale);
        assert_eq!(h.gh.api_calls(), []);
    }

    #[test]
    fn no_update_reports_a_missing_cache() {
        let mut h = Setup::new(FakeGh::new()).build();
        assert_matches!(
            load_or_refresh(&h.app, &mut h.ui, Freshness::NoUpdate),
            Err(CacheError::Missing)
        );
        assert!(!h.cache_path().exists());
    }

    // ---- Force -------------------------------------------------------------

    #[test]
    fn force_refreshes_even_a_fresh_cache() {
        let mut h = Setup::new(healthy()).build();
        h.seed(&old_cache(NOW - 60));
        let cache = load_or_refresh(&h.app, &mut h.ui, Freshness::Force { all_orgs: false })
            .expect("cache");
        assert_eq!(cache.updated_at, t(NOW));
        assert!(!slugs(&cache).contains(&"github.com/acme/old".to_owned()));
    }

    #[test]
    fn force_errors_and_keeps_the_existing_file_when_every_domain_fails() {
        let mut h = Setup::new(FakeGh::new()).build();
        h.seed(&old_cache(NOW - 60));
        let before = h.on_disk();
        assert_matches!(
            load_or_refresh(&h.app, &mut h.ui, Freshness::Force { all_orgs: false }),
            Err(CacheError::AllFailed)
        );
        assert_eq!(h.on_disk(), before);
    }

    const FAVORITES: &str = "[domains.\"github.com\"]\nfavorite_orgs = [\"acme\"]\n";

    #[test]
    fn force_honours_favourites_unless_all_orgs() {
        let mut setup = Setup::new(healthy());
        setup.config = Some(FAVORITES);
        let mut h = setup.build();
        load_or_refresh(&h.app, &mut h.ui, Freshness::Force { all_orgs: false }).expect("cache");
        assert_eq!(org_calls(&h.gh.api_calls()), [org("acme")]);

        let mut setup = Setup::new(healthy());
        setup.config = Some(FAVORITES);
        let mut h = setup.build();
        load_or_refresh(&h.app, &mut h.ui, Freshness::Force { all_orgs: true }).expect("cache");
        assert_eq!(org_calls(&h.gh.api_calls()), [org("acme"), org("widgets")]);
    }

    // ---- Multiple domains, frecency ----------------------------------------

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
        let mut setup = Setup::new(healthy());
        setup.config = Some(TWO_DOMAINS);
        let mut h = setup.build();
        h.seed(&Cache::new(
            vec![repo("acme", "old"), ghe_repo()],
            t(NOW - 2 * DAY),
        ));
        let cache = load_or_refresh(&h.app, &mut h.ui, Freshness::AutoUpdate).expect("cache");
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
