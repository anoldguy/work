//! [`App`]: the context every stage-2 command runs with.

use crate::config::{Config, Locations};
use crate::domain::StoredTime;
use crate::env::EnvSnapshot;
use crate::error::Error;
use crate::gh::Gh;
use crate::git::Git;
use crate::github::GitHubApi;
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
        let _ = (env, locations, ui);
        unimplemented!("W2.4")
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
        let Adapters { git, gh, api } = adapters;
        Self {
            env,
            locations,
            config,
            ui,
            git,
            gh,
            api,
            now,
        }
    }
}
