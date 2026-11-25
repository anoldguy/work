//! [`App`]: the context every command that needs a config runs with.

use crate::config::{self, Config, Locations};
use crate::domain::StoredTime;
use crate::env::EnvSnapshot;
use crate::error::Error;

/// Immutable process-wide context.
#[derive(Debug)]
pub struct App {
    /// The environment snapshot.
    pub env: EnvSnapshot,
    /// Derived filesystem locations.
    pub locations: Locations,
    /// The validated configuration.
    pub config: Config,
    /// "Now" for this process, read once.
    pub now: StoredTime,
}

impl App {
    /// Builds the context from `env`: the config on disk and the current time.
    ///
    /// # Errors
    ///
    /// The locations cannot be derived (no `HOME`), or the config file is
    /// unreadable or invalid.
    pub fn load(env: EnvSnapshot) -> Result<Self, Error> {
        let locations = Locations::from_env(&env)?;
        let config = config::store::load(&locations)?;
        Ok(Self {
            env,
            locations,
            config,
            now: StoredTime::now(),
        })
    }
}
