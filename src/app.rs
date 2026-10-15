//! [`App`]: the context every stage-2 command runs with.

use crate::config::{self, Config, Locations};
use crate::domain::StoredTime;
use crate::env::EnvSnapshot;
use crate::error::Error;
use crate::gh::{Gh, ProcessGh};
use crate::git::{Git, ProcessGit};
use crate::ui::Ui;

/// Process-wide context with public, disjoint fields, so a command can
/// borrow `app.ui` mutably while sharing `&*app.gh` with workers.
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
    /// The gh seam: every GitHub API call and `gh auth`.
    pub gh: Box<dyn Gh>,
    /// "Now" for this process.
    pub now: StoredTime,
}

/// The external seams, grouped so [`App::with_parts`] stays readable.
#[derive(Debug)]
pub struct Adapters {
    /// The git seam.
    pub git: Box<dyn Git>,
    /// The gh seam: every GitHub API call and `gh auth`.
    pub gh: Box<dyn Gh>,
}

impl App {
    /// Builds the real context: the config at `locations.config_file`
    /// (implicit `github.com` when none), `ProcessGit`, `ProcessGh`, and
    /// `StoredTime::now()`.
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

    /// Assembles a context from parts (tests, and `new` itself).
    #[must_use]
    pub fn with_parts(
        env: EnvSnapshot,
        locations: Locations,
        config: Config,
        ui: Ui,
        adapters: Adapters,
        now: StoredTime,
    ) -> Self {
        let Adapters { git, gh } = adapters;
        Self {
            env,
            locations,
            config,
            ui,
            git,
            gh,
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
    use crate::testing::{FakeGh, FakeGit, ScriptedInteraction, SharedBuf};

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
        };
        let app = App::with_parts(env, locations.clone(), config, test_ui(), adapters, now);
        assert_eq!(app.now, now);
        assert_eq!(app.locations, locations);
    }
}
