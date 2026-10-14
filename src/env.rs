//! A snapshot of the process environment, read once at startup.
//!
//! Nothing else in the crate calls `std::env::var*`; everything reads this
//! struct instead, which keeps XDG and token logic testable without
//! `set_var` (which is `unsafe` in edition 2024 and so forbidden here).

use std::ffi::OsString;
use std::path::PathBuf;

use crate::auth::SecretToken;

/// The environment variables `work` reads.
///
/// Empty values are captured as `None` (or `false`), following the XDG rule
/// that an empty variable counts as unset. `Debug` is safe to print: the
/// token is a [`SecretToken`], which redacts itself.
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
    /// `GITHUB_TOKEN`, used for every domain when present.
    pub github_token: Option<SecretToken>,
    /// `GITHUB_TOKEN` is set and non-empty but unusable (interior whitespace
    /// or control characters, or not UTF-8). Then `github_token` is `None`
    /// and the command must fail rather than fall back to `gh`.
    pub github_token_invalid: bool,
    /// `TERM`; `dumb` disables live progress.
    pub term: Option<String>,
    /// `HOMEBREW_PREFIX`, for completion install paths.
    pub homebrew_prefix: Option<PathBuf>,
    /// `PATH`, for locating `brew` when `HOMEBREW_PREFIX` is unset.
    pub path: Option<OsString>,
    /// `NO_COLOR` is set to a non-empty value.
    pub no_color: bool,
    /// `GIT_CONFIG_COUNT`; git children append their auth header after the
    /// user's own `GIT_CONFIG_KEY_n`/`VALUE_n` entries.
    pub git_config_count: GitConfigCount,
    /// Names of inherited variables that make git log traffic
    /// (`GIT_TRACE*` and `GIT_CURL_VERBOSE`); stripped from authenticated git
    /// children.
    pub git_trace_vars: Vec<OsString>,
}

/// The inherited `GIT_CONFIG_COUNT`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GitConfigCount {
    /// Unset or empty: entries start at index 0.
    #[default]
    Unset,
    /// A valid count: entries start at this index.
    Count(u32),
    /// Set but not a number. git itself rejects this, so no auth entries are
    /// added and git reports the problem.
    Invalid,
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
            if name.starts_with("GIT_TRACE") || name == "GIT_CURL_VERBOSE" {
                env.git_trace_vars.push(OsString::from(name));
                continue;
            }
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
                "GITHUB_TOKEN" => {
                    env.github_token = value.to_str().and_then(SecretToken::new);
                    env.github_token_invalid = env.github_token.is_none();
                }
                "TERM" => env.term = value.into_string().ok(),
                "HOMEBREW_PREFIX" => env.homebrew_prefix = Some(PathBuf::from(value)),
                "PATH" => env.path = Some(value),
                "NO_COLOR" => env.no_color = true,
                "GIT_CONFIG_COUNT" => {
                    env.git_config_count = value
                        .to_str()
                        .and_then(|text| text.parse().ok())
                        .map_or(GitConfigCount::Invalid, GitConfigCount::Count);
                }
                _ => {}
            }
        }
        env.git_trace_vars.sort();
        env
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{EnvSnapshot, GitConfigCount};

    fn snapshot(pairs: &[(&str, &str)]) -> EnvSnapshot {
        EnvSnapshot::from_vars(
            pairs
                .iter()
                .map(|(k, v)| (OsString::from(k), OsString::from(v))),
        )
    }

    #[test]
    fn reads_the_variables_work_uses() {
        let env = snapshot(&[
            ("HOME", "/home/me"),
            ("XDG_CONFIG_HOME", "/cfg"),
            ("SHELL", "/bin/zsh"),
            ("EDITOR", "vim"),
            ("WORK_EDITOR", "code -w"),
            ("GITHUB_TOKEN", "ghp_abc\n"),
            ("NO_COLOR", "1"),
            ("PATH", "/usr/bin"),
            ("UNRELATED", "x"),
        ]);
        assert_eq!(env.home, Some(PathBuf::from("/home/me")));
        assert_eq!(env.xdg_config_home, Some(PathBuf::from("/cfg")));
        assert_eq!(env.shell.as_deref(), Some("/bin/zsh"));
        assert_eq!(env.editor.as_deref(), Some("vim"));
        assert_eq!(env.work_editor.as_deref(), Some("code -w"));
        assert!(env.no_color);
        assert_eq!(env.path, Some(OsString::from("/usr/bin")));
        assert_matches!(env.github_token, Some(_));
        assert!(!format!("{env:?}").contains("ghp_abc"));
    }

    #[test]
    fn an_unusable_github_token_is_flagged_not_dropped_silently() {
        let env = snapshot(&[("GITHUB_TOKEN", "ghp abc")]);
        assert_matches!(env.github_token, None);
        assert!(env.github_token_invalid);
        let env = snapshot(&[("GITHUB_TOKEN", "ghp_abc"), ("TERM", "dumb")]);
        assert!(!env.github_token_invalid);
        assert_eq!(env.term.as_deref(), Some("dumb"));
    }

    #[test]
    fn empty_values_count_as_unset() {
        let env = snapshot(&[
            ("HOME", ""),
            ("XDG_STATE_HOME", ""),
            ("GITHUB_TOKEN", ""),
            ("NO_COLOR", ""),
            ("GIT_CONFIG_COUNT", ""),
        ]);
        assert_eq!(env.home, None);
        assert_eq!(env.xdg_state_home, None);
        assert_matches!(env.github_token, None);
        assert!(!env.no_color);
        assert_eq!(env.git_config_count, GitConfigCount::Unset);
    }

    #[test]
    fn git_config_count_is_parsed_or_flagged_invalid() {
        assert_eq!(
            snapshot(&[("GIT_CONFIG_COUNT", "3")]).git_config_count,
            GitConfigCount::Count(3)
        );
        assert_eq!(
            snapshot(&[("GIT_CONFIG_COUNT", "many")]).git_config_count,
            GitConfigCount::Invalid
        );
    }

    #[test]
    fn git_trace_variables_are_listed_by_name() {
        let env = snapshot(&[
            ("GIT_TRACE", "1"),
            ("GIT_CURL_VERBOSE", "1"),
            ("GIT_TRACE_PACKET", ""),
            ("GIT_DIR", "/x"),
        ]);
        assert_eq!(
            env.git_trace_vars,
            [
                OsString::from("GIT_CURL_VERBOSE"),
                OsString::from("GIT_TRACE"),
                OsString::from("GIT_TRACE_PACKET"),
            ]
        );
    }
}
