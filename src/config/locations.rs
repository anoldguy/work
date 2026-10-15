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
        let invalid = |reason: &'static str| ConfigError::InvalidPath {
            key,
            given: raw.to_owned(),
            reason,
        };
        let path = if raw == "~" {
            home.to_path_buf()
        } else if let Some(rest) = raw.strip_prefix("~/") {
            home.join(rest)
        } else if raw.starts_with('~') {
            return Err(invalid(
                "~user is not supported (use ~/ or an absolute path)",
            ));
        } else {
            PathBuf::from(raw)
        };
        if path.is_absolute() {
            Ok(Self(path))
        } else {
            Err(invalid("must be absolute or start with ~/"))
        }
    }

    /// Wraps a path that is already absolute; `None` for a relative path.
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
/// On macOS `XDG_CONFIG_HOME` is ignored: the config file is fixed at
/// `~/.config/work/config.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locations {
    /// `$HOME`.
    pub home: PathBuf,
    /// `$XDG_CONFIG_HOME` (not on macOS), else `~/.config`.
    pub config_home: PathBuf,
    /// `$XDG_DATA_HOME`, else `~/.local/share`.
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
        let home = env
            .home
            .as_deref()
            .filter(|home| home.is_absolute())
            .ok_or(ConfigError::NoHome)?;
        let xdg_config = if cfg!(target_os = "macos") {
            None
        } else {
            env.xdg_config_home.as_deref()
        };
        let config_home = xdg_or_default(xdg_config, home, ".config");
        let data_home = xdg_or_default(env.xdg_data_home.as_deref(), home, ".local/share");
        let state_home = xdg_or_default(env.xdg_state_home.as_deref(), home, ".local/state");
        Ok(Self {
            home: home.to_path_buf(),
            config_file: config_home.join("work").join("config.toml"),
            config_home,
            data_home,
            default_cache_dir: state_home.join("work"),
            default_work_dir: home.join("code"),
        })
    }
}

/// An XDG base directory: the variable when it is an absolute path, else
/// `home/default`. Relative values are ignored, as the XDG spec requires.
fn xdg_or_default(xdg: Option<&Path>, home: &Path, default: &str) -> PathBuf {
    match xdg.filter(|path| path.is_absolute()) {
        Some(path) => path.to_path_buf(),
        None => home.join(default),
    }
}
