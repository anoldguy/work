//! A snapshot of the process environment, read once at startup.
//!
//! Nothing else in the crate calls `std::env::var*`; everything reads this
//! struct instead, which keeps XDG and token logic testable without
//! `set_var` (which is `unsafe` in edition 2024 and so forbidden here).

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
    /// `SHELL`, used only by `activate`, `completions`, and the
    /// "shell integration is not active" hint.
    pub shell: Option<String>,
    /// `WORK_EDITOR`, the first choice for the editor.
    pub work_editor: Option<String>,
    /// `EDITOR`, the second choice for the editor.
    pub editor: Option<String>,
    /// `TERM`; `dumb` disables live progress.
    pub term: Option<String>,
    /// `HOMEBREW_PREFIX`, for completion install paths.
    pub homebrew_prefix: Option<PathBuf>,
    /// `PATH`, for locating `brew` when `HOMEBREW_PREFIX` is unset.
    pub path: Option<OsString>,
    /// `NO_COLOR` is set to a non-empty value.
    pub no_color: bool,
}

impl EnvSnapshot {
    /// Reads the process environment. The only place that does.
    #[must_use]
    pub fn capture() -> Self {
        Self::from_vars(std::env::vars_os())
    }

    /// Builds a snapshot from `vars`; `capture` feeds it the process
    /// environment and tests feed it fixed pairs.
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
