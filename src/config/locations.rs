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
/// On macOS `XDG_CONFIG_HOME` is ignored: the config file is fixed at
/// `~/.config/work/config.toml`, per `specs/configuration.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locations {
    /// `$HOME`.
    pub home: PathBuf,
    /// `$XDG_CONFIG_HOME` (not on macOS), else `~/.config` (fish completions live under it).
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

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::path::{Path, PathBuf};

    use super::{ExpandedPath, Locations};
    use crate::config::ConfigError;
    use crate::env::EnvSnapshot;

    fn with_home(home: &str) -> EnvSnapshot {
        EnvSnapshot {
            home: Some(PathBuf::from(home)),
            ..EnvSnapshot::default()
        }
    }

    #[test]
    fn defaults_follow_the_home_directory_when_xdg_is_unset() {
        let locations = Locations::from_env(&with_home("/home/me")).expect("HOME is set");
        assert_eq!(locations.home, Path::new("/home/me"));
        assert_eq!(locations.config_home, Path::new("/home/me/.config"));
        assert_eq!(
            locations.config_file,
            Path::new("/home/me/.config/work/config.toml")
        );
        assert_eq!(locations.data_home, Path::new("/home/me/.local/share"));
        assert_eq!(
            locations.default_cache_dir,
            Path::new("/home/me/.local/state/work")
        );
        assert_eq!(locations.default_work_dir, Path::new("/home/me/code"));
    }

    #[test]
    fn absolute_xdg_variables_win_over_the_defaults() {
        let env = EnvSnapshot {
            xdg_config_home: Some(PathBuf::from("/cfg")),
            xdg_state_home: Some(PathBuf::from("/state")),
            xdg_data_home: Some(PathBuf::from("/data")),
            ..with_home("/home/me")
        };
        let locations = Locations::from_env(&env).expect("HOME is set");
        assert_eq!(locations.config_home, Path::new("/cfg"));
        assert_eq!(locations.config_file, Path::new("/cfg/work/config.toml"));
        assert_eq!(locations.data_home, Path::new("/data"));
        assert_eq!(locations.default_cache_dir, Path::new("/state/work"));
        assert_eq!(locations.default_work_dir, Path::new("/home/me/code"));
    }

    #[test]
    fn relative_xdg_variables_are_ignored() {
        let env = EnvSnapshot {
            xdg_config_home: Some(PathBuf::from("relative/cfg")),
            xdg_state_home: Some(PathBuf::from("state")),
            xdg_data_home: Some(PathBuf::from("./data")),
            ..with_home("/home/me")
        };
        let locations = Locations::from_env(&env).expect("HOME is set");
        assert_eq!(locations.config_home, Path::new("/home/me/.config"));
        assert_eq!(
            locations.default_cache_dir,
            Path::new("/home/me/.local/state/work")
        );
        assert_eq!(locations.data_home, Path::new("/home/me/.local/share"));
    }

    #[test]
    fn missing_home_is_no_home() {
        assert_matches!(
            Locations::from_env(&EnvSnapshot::default()),
            Err(ConfigError::NoHome)
        );
    }

    #[test]
    fn relative_home_is_no_home() {
        assert_matches!(
            Locations::from_env(&with_home("home/me")),
            Err(ConfigError::NoHome)
        );
    }

    #[test]
    fn tilde_expands_to_home_and_tilde_slash_joins_under_it() {
        let home = Path::new("/home/me");
        let bare = ExpandedPath::expand("~", "work_dir", home).expect("~ expands");
        assert_eq!(bare.as_path(), Path::new("/home/me"));
        let nested = ExpandedPath::expand("~/code/repos", "work_dir", home).expect("~/ expands");
        assert_eq!(nested.as_path(), Path::new("/home/me/code/repos"));
    }

    #[test]
    fn absolute_paths_are_kept_as_given() {
        let expanded = ExpandedPath::expand("/srv/work", "cache_path", Path::new("/home/me"))
            .expect("absolute");
        assert_eq!(expanded.as_path(), Path::new("/srv/work"));
    }

    #[test]
    fn relative_and_user_relative_paths_are_rejected() {
        let home = Path::new("/home/me");
        assert_matches!(
            ExpandedPath::expand("code", "work_dir", home),
            Err(ConfigError::InvalidPath {
                key: "work_dir",
                ..
            })
        );
        assert_matches!(
            ExpandedPath::expand("~bob/code", "work_dir", home),
            Err(ConfigError::InvalidPath {
                key: "work_dir",
                ..
            })
        );
    }

    #[test]
    fn tilde_with_a_relative_home_is_rejected() {
        assert_matches!(
            ExpandedPath::expand("~/code", "work_dir", Path::new("home/me")),
            Err(ConfigError::InvalidPath { .. })
        );
    }

    #[test]
    fn from_absolute_rejects_relative_paths() {
        assert_matches!(ExpandedPath::from_absolute(PathBuf::from("rel")), None);
        assert_matches!(ExpandedPath::from_absolute(PathBuf::from("/abs")), Some(_));
    }
}
