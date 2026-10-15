//! The `gh` subprocess seam: every GitHub API call, plus `auth status`,
//! `login` and `logout`.
//!
//! API calls are `gh api --hostname <domain> …` children whose stdout is
//! read line by line as it arrives (`--paginate --jq` prints each page as
//! soon as gh has it). gh owns credentials, pagination and rate limits; a
//! non-zero exit is reported with gh's own stderr, reduced to one line.

pub mod repo_line;

pub use repo_line::{REPO_JQ, parse_repo_line};

use std::ffi::OsStr;
use std::fmt::Debug;
use std::io::{self, BufRead, BufReader, Read};
use std::panic::{self, AssertUnwindSafe};
use std::process::{Command, Stdio};
use std::thread;

use crate::domain::{Domain, Org, Repo};
use crate::proc;

/// What `work` asks of the GitHub CLI. `Sync` because fetch workers share
/// one `&dyn Gh` across scoped threads; implemented by [`ProcessGh`] and by
/// test fakes.
pub trait Gh: Sync + Debug {
    /// `gh api --hostname <host> user --jq .login`: the authenticated login.
    ///
    /// # Errors
    ///
    /// [`GhError::NotInstalled`], [`GhError::Spawn`], or [`GhError::Api`]
    /// on a non-zero exit or an unusable login.
    fn viewer(&self, host: &Domain) -> Result<Org, GhError>;

    /// `gh api --hostname <host> --paginate user/orgs --jq '.[].login'`.
    /// Lines that are not a valid org name are dropped.
    ///
    /// # Errors
    ///
    /// As for [`Gh::viewer`].
    fn member_orgs(&self, host: &Domain) -> Result<Vec<Org>, GhError>;

    /// The viewer's owner and collaborator repos, all pages. Calls `on_repo`
    /// once per valid repo as gh prints it, and returns the number of lines
    /// skipped because they failed validation.
    ///
    /// # Errors
    ///
    /// As for [`Gh::viewer`].
    fn user_repos(&self, host: &Domain, on_repo: &mut dyn FnMut(Repo)) -> Result<u32, GhError>;

    /// Every repo of `org` the viewer can see, all pages, reported as for
    /// [`Gh::user_repos`].
    ///
    /// # Errors
    ///
    /// As for [`Gh::viewer`].
    fn org_repos(
        &self,
        host: &Domain,
        org: &Org,
        on_repo: &mut dyn FnMut(Repo),
    ) -> Result<u32, GhError>;

    /// `gh auth status --hostname <host>` with stdio inherited, so gh prints
    /// its own report. Flush buffered output before calling it.
    ///
    /// # Errors
    ///
    /// [`GhError::NotInstalled`], or [`GhError::Failed`] on a non-zero exit
    /// (not logged in).
    fn auth_status(&self, host: &Domain) -> Result<(), GhError>;

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
    fn viewer(&self, host: &Domain) -> Result<Org, GhError> {
        viewer_via(OsStr::new(GH), host)
    }

    fn member_orgs(&self, host: &Domain) -> Result<Vec<Org>, GhError> {
        member_orgs_via(OsStr::new(GH), host)
    }

    fn user_repos(&self, host: &Domain, on_repo: &mut dyn FnMut(Repo)) -> Result<u32, GhError> {
        repos_via(OsStr::new(GH), host, &user_repos_path(), on_repo)
    }

    fn org_repos(
        &self,
        host: &Domain,
        org: &Org,
        on_repo: &mut dyn FnMut(Repo),
    ) -> Result<u32, GhError> {
        repos_via(OsStr::new(GH), host, &org_repos_path(org), on_repo)
    }

    fn auth_status(&self, host: &Domain) -> Result<(), GhError> {
        interactive_via(OsStr::new(GH), "status", host)
    }

    fn login(&self, host: &Domain) -> Result<(), GhError> {
        interactive_via(OsStr::new(GH), "login", host)
    }

    fn logout(&self, host: &Domain) -> Result<(), GhError> {
        interactive_via(OsStr::new(GH), "logout", host)
    }
}

const GH: &str = "gh";

/// The personal listing: repos the viewer owns or collaborates on.
fn user_repos_path() -> String {
    "user/repos?affiliation=owner,collaborator&per_page=100".to_owned()
}

/// Every repo of `org` (`Org` is a validated login, safe in a path).
fn org_repos_path(org: &Org) -> String {
    format!("orgs/{org}/repos?type=all&per_page=100")
}

/// A `gh` command with the environment every child gets: no update notice,
/// no pager, no colour. Captured children also get `GH_PROMPT_DISABLED`;
/// interactive ones (`auth login` needs its prompts) do not.
fn gh_command(program: &OsStr, interactive: bool) -> Command {
    let mut cmd = Command::new(program);
    cmd.env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_PAGER", "")
        .env("NO_COLOR", "1");
    if !interactive {
        cmd.env("GH_PROMPT_DISABLED", "1");
    }
    cmd
}

fn map_spawn(err: &io::Error) -> GhError {
    if err.kind() == io::ErrorKind::NotFound {
        GhError::NotInstalled
    } else {
        GhError::Spawn {
            detail: proc::one_line(err.to_string().as_bytes()),
        }
    }
}

/// gh's stderr as one line: each line sanitised by [`proc::one_line`], then
/// joined with spaces, so a two-line message ("not logged in … run gh auth
/// login") keeps its hint.
fn stderr_summary(stderr: &[u8]) -> String {
    stderr
        .split(|&b| b == b'\n')
        .map(proc::one_line)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Runs `<program> api --hostname <host> <args>`, calling `on_line` with
/// each stdout line (newline removed) as it arrives. stderr is drained on a
/// second thread so a chatty child can never block on it.
fn api_lines(
    program: &OsStr,
    host: &Domain,
    args: &[&str],
    on_line: &mut dyn FnMut(&[u8]),
) -> Result<(), GhError> {
    let mut cmd = gh_command(program, false);
    cmd.args(["api", "--hostname", host.as_str()])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| map_spawn(&e))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (read_error, stderr) = thread::scope(|scope| {
        let drain = scope.spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut stderr) = stderr {
                // A read error leaves what was read so far; it is only the
                // failure message.
                let _ = stderr.read_to_end(&mut buf);
            }
            buf
        });
        let read =
            stdout.map(|out| panic::catch_unwind(AssertUnwindSafe(|| read_lines(out, on_line))));
        // Once stdout is abandoned the child can block writing it, so stop it
        // before waiting on the stderr drain.
        let read_error = match read {
            Some(Ok(Ok(()))) | None => None,
            Some(Ok(Err(err))) => {
                let _ = child.kill();
                Some(err)
            }
            Some(Err(payload)) => {
                let _ = child.kill();
                let _ = drain.join();
                panic::resume_unwind(payload);
            }
        };
        (read_error, drain.join().unwrap_or_default())
    });
    let status = child.wait().map_err(|e| map_spawn(&e))?;
    if let Some(err) = read_error {
        return Err(GhError::Api {
            host: host.clone(),
            detail: format!(
                "reading gh output failed: {}",
                proc::one_line(err.to_string().as_bytes())
            ),
        });
    }
    if status.success() {
        return Ok(());
    }
    let summary = stderr_summary(&stderr);
    Err(GhError::Api {
        host: host.clone(),
        detail: if summary.is_empty() {
            format!("gh api exited with {status}")
        } else {
            summary
        },
    })
}

/// Feeds every line of `out` to `on_line` without its line ending.
fn read_lines(out: impl Read, on_line: &mut dyn FnMut(&[u8])) -> io::Result<()> {
    let mut reader = BufReader::new(out);
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        let trimmed = line.strip_suffix(b"\n").unwrap_or(&line);
        on_line(trimmed.strip_suffix(b"\r").unwrap_or(trimmed));
    }
}

fn viewer_via(program: &OsStr, host: &Domain) -> Result<Org, GhError> {
    let mut lines = Vec::new();
    api_lines(program, host, &["user", "--jq", ".login"], &mut |line| {
        lines.push(String::from_utf8_lossy(line).trim().to_owned());
    })?;
    let login = lines
        .into_iter()
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    Org::try_from(login.clone()).map_err(|_| GhError::Api {
        host: host.clone(),
        detail: format!("gh printed an unusable login {login:?}"),
    })
}

fn member_orgs_via(program: &OsStr, host: &Domain) -> Result<Vec<Org>, GhError> {
    let mut orgs = Vec::new();
    api_lines(
        program,
        host,
        &["--paginate", "user/orgs", "--jq", ".[].login"],
        &mut |line| {
            let name = String::from_utf8_lossy(line).trim().to_owned();
            if let Ok(org) = Org::try_from(name) {
                orgs.push(org);
            }
        },
    )?;
    Ok(orgs)
}

/// Streams a repo listing at `path`; returns the number of skipped lines.
fn repos_via(
    program: &OsStr,
    host: &Domain,
    path: &str,
    on_repo: &mut dyn FnMut(Repo),
) -> Result<u32, GhError> {
    let mut skipped: u32 = 0;
    api_lines(
        program,
        host,
        &["--paginate", path, "--jq", REPO_JQ],
        &mut |line| {
            if line.iter().all(u8::is_ascii_whitespace) {
                return;
            }
            match parse_repo_line(line, host) {
                Some(repo) => on_repo(repo),
                None => skipped = skipped.saturating_add(1),
            }
        },
    )?;
    Ok(skipped)
}

/// Runs `<program> auth <action> --hostname <host>` with inherited stdio.
fn interactive_via(program: &OsStr, action: &'static str, host: &Domain) -> Result<(), GhError> {
    let mut cmd = gh_command(program, true);
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

/// A failure running `gh`. `Clone` so test fakes can script it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GhError {
    /// `gh` is not on `PATH`.
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
        /// `status`, `login`, or `logout`.
        action: &'static str,
        /// The host it ran against.
        host: Domain,
        /// gh's stderr through `proc::one_line`, or the exit status for
        /// inherited-stdio runs.
        detail: String,
    },
    /// `gh api` exited non-zero (or printed something unusable). The
    /// message is gh's own, which names the host and the fix; `work` adds
    /// nothing to it.
    #[error("{detail}")]
    Api {
        /// The host it ran against.
        host: Domain,
        /// gh's stderr reduced to one line.
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

    const LINE_A: &str = r#"{"name":"a","owner":"acme","clone_url":"https://github.com/acme/a.git","html_url":"https://github.com/acme/a"}"#;
    const LINE_B: &str = r#"{"name":"b","owner":"acme","clone_url":"https://github.com/acme/b.git","html_url":"https://github.com/acme/b"}"#;

    #[test]
    fn org_repos_stream_valid_lines_and_count_the_rest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = dir.path().join("argv");
        let body = format!(
            "printf '%s\\n' \"$@\" > '{}'\nenv | grep -E '^(GH_|NO_COLOR)' | sort >> '{}'\n\
             echo '{LINE_A}'\necho 'garbage'\necho\necho '{LINE_B}'",
            log.display(),
            log.display()
        );
        let gh = shim(dir.path(), "gh", &body);
        let mut seen = Vec::new();
        let skipped = repos_via(
            gh.as_os_str(),
            &host(),
            &org_repos_path(&"acme".parse().expect("org")),
            &mut |r| seen.push(r.slug.to_string()),
        )
        .expect("ok");
        assert_eq!(seen, ["github.com/acme/a", "github.com/acme/b"]);
        assert_eq!(skipped, 1);
        let argv = std::fs::read_to_string(&log).expect("log");
        let lines: Vec<&str> = argv.lines().collect();
        assert_eq!(
            lines[..7],
            [
                "api",
                "--hostname",
                "github.com",
                "--paginate",
                "orgs/acme/repos?type=all&per_page=100",
                "--jq",
                REPO_JQ
            ]
        );
        for var in [
            "GH_NO_UPDATE_NOTIFIER=1",
            "GH_PAGER=",
            "GH_PROMPT_DISABLED=1",
            "NO_COLOR=1",
        ] {
            assert!(lines.contains(&var), "{var} missing from {lines:?}");
        }
    }

    #[test]
    fn a_failed_call_carries_ghs_stderr_on_one_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let gh = shim(
            dir.path(),
            "gh",
            "printf '{\"message\":\"Not Found\"}'\n\
             echo 'To get started with GitHub CLI, please run:  gh auth login' >&2\n\
             echo 'Alternatively, populate the GH_TOKEN environment variable.' >&2\nexit 4",
        );
        let err = repos_via(gh.as_os_str(), &host(), &user_repos_path(), &mut |_| {}).unwrap_err();
        let text = err.to_string();
        assert!(!text.contains('\n'), "{text:?}");
        assert!(text.contains("gh auth login"), "{text}");
        assert!(text.contains("GH_TOKEN"), "{text}");
        assert_matches!(
            viewer_via(gh.as_os_str(), &host()),
            Err(GhError::Api { .. })
        );
    }

    #[test]
    fn viewer_and_member_orgs_parse_logins() {
        let dir = tempfile::tempdir().expect("tempdir");
        let gh = shim(
            dir.path(),
            "gh",
            r#"case "$4" in user) echo octocat ;; --paginate) printf 'acme\n-bad\nwidgets\n' ;; esac"#,
        );
        let viewer = viewer_via(gh.as_os_str(), &host()).expect("viewer");
        assert_eq!(viewer.as_str(), "octocat");
        let orgs = member_orgs_via(gh.as_os_str(), &host()).expect("orgs");
        let names: Vec<&str> = orgs.iter().map(Org::as_str).collect();
        assert_eq!(names, ["acme", "widgets"]);
        let empty = shim(dir.path(), "empty", "true");
        assert_matches!(
            viewer_via(empty.as_os_str(), &host()),
            Err(GhError::Api { .. })
        );
    }

    #[test]
    fn missing_gh_is_not_installed() {
        let missing = OsStr::new("/nonexistent/work-no-gh");
        assert_matches!(viewer_via(missing, &host()), Err(GhError::NotInstalled));
        assert_matches!(
            interactive_via(missing, "login", &host()),
            Err(GhError::NotInstalled)
        );
    }

    #[test]
    fn interactive_actions_pass_hostname_and_report_exit_status() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ok = shim(
            dir.path(),
            "ok",
            r#"[ "$1 $2 $3 $4" = "auth status --hostname github.com" ]"#,
        );
        assert_matches!(interactive_via(ok.as_os_str(), "status", &host()), Ok(()));
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
