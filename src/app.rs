//! [`App`]: the context every command that needs a config runs with.

use crate::config::{self, Config, Locations};
use crate::domain::StoredTime;
use crate::env::EnvSnapshot;
use crate::error::Error;
use crate::gh::{Gh, ProcessGh};
use crate::git::{Git, ProcessGit};

/// Immutable process-wide context.
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
    /// The gh seam.
    pub gh: Box<dyn Gh>,
    /// "Now" for this process, read once.
    pub now: StoredTime,
}

/// The external seams an [`App`] runs with.
#[derive(Debug)]
pub struct Adapters {
    /// The git seam.
    pub git: Box<dyn Git>,
    /// The gh seam.
    pub gh: Box<dyn Gh>,
}

impl App {
    /// Builds the real context from `env`: the config on disk, real `git`
    /// and `gh`, and the current time.
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

    /// Assembles a context from parts.
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
