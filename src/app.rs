//! [`App`]: the context every stage-2 command runs with.

use crate::config::{self, Config, Locations};
use crate::domain::StoredTime;
use crate::env::EnvSnapshot;
use crate::error::Error;
use crate::gh::{Gh, MemoGh, ProcessGh};
use crate::git::{Git, ProcessGit};
use crate::github::{GitHubApi, HttpGitHub};
use crate::ui::Ui;

/// Process-wide context with public, disjoint fields, so a command can
/// borrow `app.ui` mutably while sharing `&*app.api` with workers.
///
/// `now` is read once here and passed down; nothing else reads the clock.
/// `Debug` is safe to print (the token redacts itself).
#[derive(Debug)]
pub struct App {
    /// The environment snapshot.
    pub env: EnvSnapshot,
    /// Derived filesystem locations.
    pub locations: Locations,
    /// The validated configuration.
    pub config: Config,
    /// The sole terminal owner.
    pub ui: Ui,
    /// The git seam.
    pub git: Box<dyn Git>,
    /// The gh seam.
    pub gh: Box<dyn Gh>,
    /// The GitHub API seam.
    pub api: Box<dyn GitHubApi>,
    /// "Now" for this process.
    pub now: StoredTime,
}

/// The three external seams, grouped so [`App::with_parts`] stays readable.
#[derive(Debug)]
pub struct Adapters {
    /// The git seam.
    pub git: Box<dyn Git>,
    /// The gh seam.
    pub gh: Box<dyn Gh>,
    /// The GitHub API seam.
    pub api: Box<dyn GitHubApi>,
}

impl App {
    /// Builds the real context: the config at `locations.config_file`
    /// (implicit `github.com` when none), `ProcessGit`, `ProcessGh`,
    /// `HttpGitHub` with the configured API bases, and `StoredTime::now()`.
    /// `lib::run` computes `locations` once and also uses it for stage-1
    /// commands.
    ///
    /// # Errors
    ///
    /// The config file is unreadable or invalid.
    pub fn new(env: EnvSnapshot, locations: Locations, ui: Ui) -> Result<Self, Error> {
        let config = config::store::load(&locations)?;
        let adapters = Adapters {
            git: Box::new(ProcessGit::new()),
            gh: Box::new(ProcessGh),
            api: Box::new(HttpGitHub::new(config.api_bases())),
        };
        Ok(Self::with_parts(
            env,
            locations,
            config,
            ui,
            adapters,
            StoredTime::now(),
        ))
    }

    /// Assembles a context from parts (tests, and `new` itself). The gh seam
    /// is wrapped in [`MemoGh`], so a domain's token comes from `gh` at most
    /// once per process.
    #[must_use]
    pub fn with_parts(
        env: EnvSnapshot,
        locations: Locations,
        config: Config,
        ui: Ui,
        adapters: Adapters,
        now: StoredTime,
    ) -> Self {
        let Adapters { git, gh, api } = adapters;
        Self {
            env,
            locations,
            config,
            ui,
            git,
            gh: Box::new(MemoGh::new(gh)),
            api,
            now,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::path::Path;

    use super::*;
    use crate::config::ConfigError;
    use crate::testing::{FakeGh, FakeGit, NullGitHub, ScriptedInteraction, SharedBuf};

    fn env_with_home(home: &Path) -> EnvSnapshot {
        EnvSnapshot {
            home: Some(home.to_path_buf()),
            ..EnvSnapshot::default()
        }
    }

    fn test_ui() -> Ui {
        Ui::for_test(
            Box::new(SharedBuf::new()),
            Box::new(SharedBuf::new()),
            Box::new(ScriptedInteraction::new(false, vec![])),
        )
    }

    #[test]
    fn new_without_config_uses_implicit_github() {
        let dir = tempfile::tempdir().expect("tempdir");
        let env = env_with_home(dir.path());
        let locations = Locations::from_env(&env).expect("locations");
        let app = App::new(env, locations, test_ui()).expect("app");
        assert_eq!(
            app.config
                .default_domain()
                .map(crate::domain::Domain::as_str),
            Some("github.com")
        );
        assert_eq!(app.config.work_dir(), dir.path().join("code"));
    }

    #[test]
    fn new_reports_an_invalid_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let env = env_with_home(dir.path());
        let locations = Locations::from_env(&env).expect("locations");
        let parent = locations.config_file.parent().expect("parent");
        std::fs::create_dir_all(parent).expect("mkdir");
        std::fs::write(&locations.config_file, "[domains.github.com]\n").expect("write");
        let err = App::new(env, locations, test_ui()).expect_err("invalid");
        assert_matches!(err, Error::Config(ConfigError::NestedDomainTable { .. }));
    }

    #[test]
    fn with_parts_keeps_what_it_is_given() {
        let dir = tempfile::tempdir().expect("tempdir");
        let env = env_with_home(dir.path());
        let locations = Locations::from_env(&env).expect("locations");
        let config = config::store::load(&locations).expect("config");
        let now = StoredTime::from_unix_seconds(1_700_000_000).expect("time");
        let adapters = Adapters {
            git: Box::new(FakeGit::new()),
            gh: Box::new(FakeGh::new()),
            api: Box::new(NullGitHub),
        };
        let app = App::with_parts(env, locations.clone(), config, test_ui(), adapters, now);
        assert_eq!(app.now, now);
        assert_eq!(app.locations, locations);
    }

    #[test]
    fn gh_token_is_fetched_once_per_process_but_failures_are_retried() {
        use crate::auth::{EnvToken, Tokens};
        use crate::domain::Domain;
        use std::sync::Arc;

        let dir = tempfile::tempdir().expect("tempdir");
        let env = env_with_home(dir.path());
        let locations = Locations::from_env(&env).expect("locations");
        let config = config::store::load(&locations).expect("config");
        let now = StoredTime::from_unix_seconds(1_700_000_000).expect("time");
        let github: Domain = "github.com".parse().expect("domain");
        let ghe: Domain = "ghe.example.com".parse().expect("domain");
        let gh = Arc::new(
            FakeGh::new()
                .with_token(github.clone(), "gho_a")
                .with_token_after_login(ghe.clone(), "gho_b"),
        );
        let adapters = Adapters {
            git: Box::new(FakeGit::new()),
            gh: Box::new(Arc::clone(&gh)),
            api: Box::new(NullGitHub),
        };
        let app = App::with_parts(env, locations, config, test_ui(), adapters, now);
        let both = [github.clone(), ghe.clone()];
        for _ in 0..3 {
            let tokens = Tokens::resolve(&*app.gh, &EnvToken::Unset, &both);
            assert_matches!(tokens.get(&github), Ok(_));
            assert_matches!(tokens.get(&ghe), Err(_));
        }
        app.gh.login(&ghe).expect("login");
        let tokens = Tokens::resolve(&*app.gh, &EnvToken::Unset, &both);
        assert_matches!(tokens.get(&ghe), Ok(_));
        let count = |host: &Domain, action: &str| {
            gh.calls()
                .iter()
                .filter(|c| c.host == *host && c.action == action)
                .count()
        };
        assert_eq!(count(&github, "token"), 1);
        assert_eq!(count(&ghe, "token"), 4);
    }
}
