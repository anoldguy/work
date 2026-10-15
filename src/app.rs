//! [`App`]: the context every command that needs a config runs with.

use crate::config::{self, Config, Locations};
use crate::domain::StoredTime;
use crate::env::EnvSnapshot;
use crate::error::Error;
use crate::gh::{Gh, ProcessGh};
use crate::git::{Git, ProcessGit};

/// Immutable process-wide context. Commands take `&mut Ui` next to it.
///
/// `now` is read once here and passed down; nothing else reads the clock.
#[derive(Debug)]
pub struct App {
    /// The environment snapshot.
    pub env: EnvSnapshot,
    /// Derived filesystem locations.
    pub locations: Locations,
    /// The validated configuration.
    pub config: Config,
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
    /// Builds the real context: `Locations` from `env`, the config at
    /// `locations.config_file` (implicit `github.com` when none),
    /// `ProcessGit`, `ProcessGh`, and `StoredTime::now()`.
    ///
    /// # Errors
    ///
    /// The locations cannot be derived (no `HOME`), or the config file is
    /// unreadable or invalid.
    pub fn load(env: EnvSnapshot) -> Result<Self, Error> {
        let locations = Locations::from_env(&env)?;
        let config = config::store::load(&locations)?;
        let adapters = Adapters {
            git: Box::new(ProcessGit::new()),
            gh: Box::new(ProcessGh),
        };
        Ok(Self::with_parts(
            env,
            locations,
            config,
            adapters,
            StoredTime::now(),
        ))
    }

    /// Assembles a context from parts (tests, and `load` itself).
    #[must_use]
    pub fn with_parts(
        env: EnvSnapshot,
        locations: Locations,
        config: Config,
        adapters: Adapters,
        now: StoredTime,
    ) -> Self {
        let Adapters { git, gh } = adapters;
        Self {
            env,
            locations,
            config,
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
    use crate::testing::{FakeGh, FakeGit};

    fn env_with_home(home: &Path) -> EnvSnapshot {
        EnvSnapshot {
            home: Some(home.to_path_buf()),
            ..EnvSnapshot::default()
        }
    }

    #[test]
    fn new_without_config_uses_implicit_github() {
        let dir = tempfile::tempdir().expect("tempdir");
        let env = env_with_home(dir.path());
        let app = App::load(env).expect("app");
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
        let err = App::load(env).expect_err("invalid");
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
        let app = App::with_parts(env, locations.clone(), config, adapters, now);
        assert_eq!(app.now, now);
        assert_eq!(app.locations, locations);
    }
}
