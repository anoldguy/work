//! A snapshot of the process environment, read once at startup.

use std::ffi::OsString;
use std::path::PathBuf;

/// The environment variables `work` reads.
///
/// Empty values are captured as `None` (or `false`), following the XDG rule
/// that an empty variable counts as unset.
#[derive(Debug, Default)]
pub struct EnvSnapshot {
    /// `HOME`.
    pub home: Option<PathBuf>,
    /// `XDG_CONFIG_HOME`.
    pub xdg_config_home: Option<PathBuf>,
    /// `XDG_STATE_HOME`.
    pub xdg_state_home: Option<PathBuf>,
    /// `XDG_DATA_HOME`.
    pub xdg_data_home: Option<PathBuf>,
    /// `SHELL`.
    pub shell: Option<String>,
    /// `WORK_EDITOR`.
    pub work_editor: Option<String>,
    /// `EDITOR`.
    pub editor: Option<String>,
    /// `TERM`.
    pub term: Option<String>,
    /// `HOMEBREW_PREFIX`.
    pub homebrew_prefix: Option<PathBuf>,
    /// `PATH`.
    pub path: Option<OsString>,
    /// `NO_COLOR` is set to a non-empty value.
    pub no_color: bool,
}

impl EnvSnapshot {
    /// Reads the process environment.
    #[must_use]
    pub fn capture() -> Self {
        Self::from_vars(std::env::vars_os())
    }

    fn from_vars(vars: impl IntoIterator<Item = (OsString, OsString)>) -> Self {
        let mut env = Self::default();
        for (name, value) in vars {
            let Some(name) = name.to_str() else { continue };
            if value.is_empty() {
                continue;
            }
            match name {
                "HOME" => env.home = Some(PathBuf::from(value)),
                "XDG_CONFIG_HOME" => env.xdg_config_home = Some(PathBuf::from(value)),
                "XDG_STATE_HOME" => env.xdg_state_home = Some(PathBuf::from(value)),
                "XDG_DATA_HOME" => env.xdg_data_home = Some(PathBuf::from(value)),
                "SHELL" => env.shell = value.into_string().ok(),
                "WORK_EDITOR" => env.work_editor = value.into_string().ok(),
                "EDITOR" => env.editor = value.into_string().ok(),
                "TERM" => env.term = value.into_string().ok(),
                "HOMEBREW_PREFIX" => env.homebrew_prefix = Some(PathBuf::from(value)),
                "PATH" => env.path = Some(value),
                "NO_COLOR" => env.no_color = true,
                _ => {}
            }
        }
        env
    }
}
