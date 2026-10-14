//! The `gh` subprocess seam: token lookup, login, logout.

use std::ffi::OsStr;
use std::fmt::Debug;
use std::io;
use std::process::Command;

use crate::auth::SecretToken;
use crate::domain::Domain;
use crate::proc;

/// What `work` asks of the GitHub CLI. `Sync` so it can sit in `App` beside
/// the other seams; implemented by [`ProcessGh`] and by test fakes.
pub trait Gh: Sync + Debug {
    /// `gh auth token --hostname <host>`. `Ok(None)` when gh has no token
    /// for the host.
    ///
    /// # Errors
    ///
    /// [`GhError::NotInstalled`] when `gh` is not on `PATH`, or
    /// [`GhError::Failed`] for any other failure.
    fn auth_token(&self, host: &Domain) -> Result<Option<SecretToken>, GhError>;

    /// `gh auth login --hostname <host>` with stdio inherited (interactive).
    ///
    /// # Errors
    ///
    /// [`GhError::NotInstalled`], or [`GhError::Failed`] on a non-zero exit.
    fn login(&self, host: &Domain) -> Result<(), GhError>;

    /// `gh auth logout --hostname <host>` with stdio inherited.
    ///
    /// # Errors
    ///
    /// [`GhError::NotInstalled`], or [`GhError::Failed`] on a non-zero exit.
    fn logout(&self, host: &Domain) -> Result<(), GhError>;
}

/// The real `gh`, run as a child process found on `PATH`.
#[derive(Debug, Default)]
pub struct ProcessGh;

impl Gh for ProcessGh {
    fn auth_token(&self, host: &Domain) -> Result<Option<SecretToken>, GhError> {
        token_via(OsStr::new(GH), host)
    }

    fn login(&self, host: &Domain) -> Result<(), GhError> {
        interactive_via(OsStr::new(GH), "login", host)
    }

    fn logout(&self, host: &Domain) -> Result<(), GhError> {
        interactive_via(OsStr::new(GH), "logout", host)
    }
}

const GH: &str = "gh";

fn map_spawn(err: &io::Error) -> GhError {
    if err.kind() == io::ErrorKind::NotFound {
        GhError::NotInstalled
    } else {
        GhError::Spawn {
            detail: proc::one_line(err.to_string().as_bytes()),
        }
    }
}

/// Runs `<program> auth token --hostname <host>`.
fn token_via(program: &OsStr, host: &Domain) -> Result<Option<SecretToken>, GhError> {
    let mut cmd = Command::new(program);
    cmd.args(["auth", "token", "--hostname", host.as_str()]);
    let out = proc::run_capture(&mut cmd).map_err(|e| map_spawn(&e))?;
    if out.status.success() {
        let text = std::str::from_utf8(&out.stdout).unwrap_or_default();
        if text.trim().is_empty() {
            return Ok(None);
        }
        return SecretToken::new(text)
            .map(Some)
            .ok_or_else(|| GhError::Failed {
                action: "token",
                host: host.clone(),
                detail: "gh printed an unusable token".to_owned(),
            });
    }
    let mut detail = proc::one_line(&out.stderr);
    let lower = detail.to_lowercase();
    if lower.contains("no oauth token") || lower.contains("not logged in") {
        return Ok(None);
    }
    if lower.contains("unknown command") {
        detail.push_str(" (upgrade gh to 2.17 or later)");
    }
    Err(GhError::Failed {
        action: "token",
        host: host.clone(),
        detail,
    })
}

/// Runs `<program> auth <action> --hostname <host>` with inherited stdio.
fn interactive_via(program: &OsStr, action: &'static str, host: &Domain) -> Result<(), GhError> {
    let mut cmd = Command::new(program);
    cmd.args(["auth", action, "--hostname", host.as_str()]);
    let status = proc::run_inherit(&mut cmd).map_err(|e| map_spawn(&e))?;
    if status.success() {
        Ok(())
    } else {
        Err(GhError::Failed {
            action,
            host: host.clone(),
            detail: status.to_string(),
        })
    }
}

/// A failure running `gh`. `Clone` so it can live inside a cloned
/// `AuthError`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GhError {
    /// `gh` is not on `PATH`. Only possible when `GITHUB_TOKEN` is unset.
    #[error("GitHub CLI 'gh' is required (install from https://cli.github.com)")]
    NotInstalled,
    /// `gh` could not be started for another reason.
    #[error("could not run gh: {detail}")]
    Spawn {
        /// The OS error, one line.
        detail: String,
    },
    /// `gh auth <action>` ran and failed.
    #[error("gh auth {action} failed for {host}: {detail}")]
    Failed {
        /// `token`, `login`, or `logout`.
        action: &'static str,
        /// The host it ran against.
        host: Domain,
        /// gh's stderr through `proc::one_line` and `auth::redact`, or the
        /// exit status for inherited-stdio runs.
        detail: String,
    },
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::path::{Path, PathBuf};

    use super::*;

    fn host() -> Domain {
        "github.com".parse().expect("valid domain")
    }

    /// Installs an executable `#!/bin/sh` shim, safe to exec at once (no
    /// `ETXTBSY` under the threaded test runner).
    fn shim(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        crate::testing::install_script(&path, body);
        path
    }

    #[test]
    fn token_is_read_from_stdout_and_trimmed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let gh = shim(
            dir.path(),
            "gh",
            r#"[ "$1 $2 $3 $4" = "auth token --hostname github.com" ] && echo gho_abc"#,
        );
        let token = token_via(gh.as_os_str(), &host())
            .expect("ok")
            .expect("some");
        assert_eq!(token.expose(), "gho_abc");
    }

    #[test]
    fn unusable_token_output_is_an_error_not_no_token() {
        let dir = tempfile::tempdir().expect("tempdir");
        let gh = shim(dir.path(), "gh", "echo 'two words'");
        assert_matches!(
            token_via(gh.as_os_str(), &host()),
            Err(GhError::Failed { detail, .. }) if detail.contains("unusable")
        );
    }

    #[test]
    fn old_gh_without_auth_token_gets_an_upgrade_hint() {
        let dir = tempfile::tempdir().expect("tempdir");
        let gh = shim(dir.path(), "gh", "echo 'unknown command' >&2; exit 1");
        assert_matches!(
            token_via(gh.as_os_str(), &host()),
            Err(GhError::Failed { detail, .. }) if detail.contains("upgrade gh")
        );
    }

    #[test]
    fn no_oauth_token_is_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let gh = shim(
            dir.path(),
            "gh",
            "echo 'no oauth token found for github.com' >&2; exit 1",
        );
        assert_matches!(token_via(gh.as_os_str(), &host()), Ok(None));
    }

    #[test]
    fn other_failure_is_one_line_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let gh = shim(dir.path(), "gh", "echo boom >&2; exit 2");
        let err = token_via(gh.as_os_str(), &host()).unwrap_err();
        assert_matches!(&err, GhError::Failed { action: "token", detail, .. } if detail == "boom");
        assert!(!err.to_string().contains('\n'));
    }

    #[test]
    fn missing_gh_is_not_installed() {
        let missing = OsStr::new("/nonexistent/work-no-gh");
        assert_matches!(token_via(missing, &host()), Err(GhError::NotInstalled));
        assert_matches!(
            interactive_via(missing, "login", &host()),
            Err(GhError::NotInstalled)
        );
        assert!(GhError::NotInstalled.to_string().contains("required"));
    }

    #[test]
    fn login_passes_hostname_and_reports_exit_status() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ok = shim(
            dir.path(),
            "ok",
            r#"[ "$1 $2 $3 $4" = "auth login --hostname github.com" ]"#,
        );
        assert_matches!(interactive_via(ok.as_os_str(), "login", &host()), Ok(()));
        let bad = shim(dir.path(), "bad", "exit 3");
        assert_matches!(
            interactive_via(bad.as_os_str(), "logout", &host()),
            Err(GhError::Failed {
                action: "logout",
                ..
            })
        );
    }
}
