//! XDG locations and `~` expansion, computed from `EnvSnapshot`.

use std::path::{Path, PathBuf};

use crate::config::ConfigError;
use crate::env::EnvSnapshot;

/// An absolute path, after expanding a leading `~` or `~/`.
///
/// Invariant: absolute. `~user` forms are rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedPath(PathBuf);

impl ExpandedPath {
    /// Expands `raw` against `home`. `key` names the setting for errors.
    ///
    /// # Errors
    ///
    /// [`ConfigError::InvalidPath`] when the result is not absolute or `raw`
    /// uses `~user`.
    pub fn expand(raw: &str, key: &'static str, home: &Path) -> Result<Self, ConfigError> {
        let _ = (raw, key, home);
        unimplemented!("W1.12")
    }

    /// Wraps a path that is already absolute (for defaults and tests).
    /// Returns `None` for a relative path.
    #[must_use]
    pub fn from_absolute(path: PathBuf) -> Option<Self> {
        path.is_absolute().then_some(Self(path))
    }

    /// The path.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for ExpandedPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

/// Every filesystem location `work` derives from the environment.
///
/// XDG variables win when set (and non-empty), else the `~`-based defaults.
/// The same layout is used on macOS, per `specs/configuration.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locations {
    /// `$HOME`.
    pub home: PathBuf,
    /// `$XDG_CONFIG_HOME`, else `~/.config` (fish completions live under it).
    pub config_home: PathBuf,
    /// `$XDG_DATA_HOME`, else `~/.local/share` (bash/zsh completions).
    pub data_home: PathBuf,
    /// `{config_home}/work/config.toml`.
    pub config_file: PathBuf,
    /// `$XDG_STATE_HOME/work`, else `~/.local/state/work`: the cache
    /// directory when `cache_path` is not configured.
    pub default_cache_dir: PathBuf,
    /// `~/code`: the work directory when `work_dir` is not configured.
    pub default_work_dir: PathBuf,
}

impl Locations {
    /// Computes every location from `env`.
    ///
    /// # Errors
    ///
    /// [`ConfigError::NoHome`] when `HOME` is unset.
    pub fn from_env(env: &EnvSnapshot) -> Result<Self, ConfigError> {
        let _ = env;
        unimplemented!("W1.12")
    }
}
