//! GitHub access through the `gh` CLI.

pub mod repo_line;

pub use repo_line::{REPO_JQ, REPO_OBJECT_JQ, parse_repo_line};

use std::ffi::OsStr;
use std::fmt::Debug;
use std::io::{self, BufRead, BufReader, Read};
use std::panic::{self, AssertUnwindSafe};
use std::process::Command;
use std::thread;

use crate::domain::{Domain, Org, Repo, RepoName};
use crate::proc::{self, Interactive};
use crate::text;

/// What `work` asks of the GitHub CLI.
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

    /// `gh api --hostname <host> repos/<org>/<name>`: the repo under its
    /// current owner and name (gh follows redirects), validated like a
    /// listing entry.
    ///
    /// # Errors
    ///
    /// As for [`Gh::viewer`], for every failure other than HTTP 401, 403 or
    /// 404, and for an answer that fails validation.
    fn repo(&self, host: &Domain, org: &Org, name: &RepoName) -> Result<Lookup, GhError>;

    /// `gh auth status --hostname <host>` on `term`, so gh prints its own
    /// report.
    ///
    /// # Errors
    ///
    /// [`GhError::NotInstalled`], or [`GhError::Failed`] on a non-zero exit
    /// (not logged in).
    fn auth_status(&self, term: &mut Interactive<'_>, host: &Domain) -> Result<(), GhError>;

    /// `gh auth login --hostname <host>` on `term` (interactive).
    ///
    /// # Errors
    ///
    /// [`GhError::NotInstalled`], or [`GhError::Failed`] on a non-zero exit.
    fn login(&self, term: &mut Interactive<'_>, host: &Domain) -> Result<(), GhError>;

    /// `gh auth logout --hostname <host>` on `term`.
    ///
    /// # Errors
    ///
    /// [`GhError::NotInstalled`], or [`GhError::Failed`] on a non-zero exit.
    fn logout(&self, term: &mut Interactive<'_>, host: &Domain) -> Result<(), GhError>;
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

    fn repo(&self, host: &Domain, org: &Org, name: &RepoName) -> Result<Lookup, GhError> {
        repo_via(OsStr::new(GH), host, org, name)
    }

    fn auth_status(&self, term: &mut Interactive<'_>, host: &Domain) -> Result<(), GhError> {
        interactive_via(term, OsStr::new(GH), "status", host)
    }

    fn login(&self, term: &mut Interactive<'_>, host: &Domain) -> Result<(), GhError> {
        interactive_via(term, OsStr::new(GH), "login", host)
    }

    fn logout(&self, term: &mut Interactive<'_>, host: &Domain) -> Result<(), GhError> {
        interactive_via(term, OsStr::new(GH), "logout", host)
    }
}

const GH: &str = "gh";

fn user_repos_path() -> String {
    "user/repos?affiliation=owner,collaborator&per_page=100".to_owned()
}

fn org_repos_path(org: &Org) -> String {
    format!("orgs/{org}/repos?type=all&per_page=100")
}

/// Interactive children keep gh's prompts, which `auth login` needs.
fn gh_command(program: &OsStr, interactive: bool) -> Command {
    let mut cmd = proc::command(program);
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
            detail: text::one_line(&err.to_string()),
        }
    }
}

/// stderr is drained on a second thread so a chatty child can never block on it.
fn api_lines(
    program: &OsStr,
    host: &Domain,
    args: &[&str],
    on_line: &mut dyn FnMut(&[u8]),
) -> Result<(), GhError> {
    let mut cmd = gh_command(program, false);
    cmd.args(["api", "--hostname", host.as_str()]).args(args);
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
                text::one_line(&err.to_string())
            ),
        });
    }
    if status.success() {
        return Ok(());
    }
    let summary = proc::stderr_summary(&stderr, proc::Fallback::AllLines);
    Err(GhError::Api {
        host: host.clone(),
        detail: if summary.is_empty() {
            format!("gh api exited with {status}")
        } else {
            summary
        },
    })
}

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

fn repo_via(program: &OsStr, host: &Domain, org: &Org, name: &RepoName) -> Result<Lookup, GhError> {
    let path = format!("repos/{org}/{name}");
    let mut lines = Vec::new();
    let answered = api_lines(
        program,
        host,
        &[&path, "--jq", REPO_OBJECT_JQ],
        &mut |line| {
            lines.push(line.to_vec());
        },
    );
    match answered {
        Ok(()) => {}
        Err(GhError::Api { detail, host }) => {
            return match http_status(&detail) {
                Some(404) => Ok(Lookup::NotFound),
                Some(401 | 403) if !is_rate_limit(&detail) => Ok(Lookup::Denied(detail)),
                _ => Err(GhError::Api { host, detail }),
            };
        }
        Err(err) => return Err(err),
    }
    lines
        .iter()
        .find(|line| !line.iter().all(u8::is_ascii_whitespace))
        .and_then(|line| parse_repo_line(line, host))
        .map(Lookup::Found)
        .ok_or_else(|| GhError::Api {
            host: host.clone(),
            detail: format!("gh returned an unusable answer for {path}"),
        })
}

/// The last `HTTP NNN` in gh's error text: gh prints `<message> (HTTP 404)`
/// when the body carries a message, and plain `HTTP 404` otherwise.
fn http_status(detail: &str) -> Option<u16> {
    detail
        .match_indices("HTTP ")
        .filter(|&(i, _)| {
            !detail[..i]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric)
        })
        .filter_map(|(i, word)| {
            let rest = &detail[i + word.len()..];
            let code = rest.get(..3)?;
            let bounded = !rest[3..].starts_with(|c: char| c.is_ascii_alphanumeric());
            (bounded && code.bytes().all(|b| b.is_ascii_digit()))
                .then(|| code.parse().ok())
                .flatten()
        })
        .last()
}

/// A 403 for a primary or secondary rate limit is not about access.
fn is_rate_limit(detail: &str) -> bool {
    detail.to_ascii_lowercase().contains("rate limit")
}

fn interactive_via(
    term: &mut Interactive<'_>,
    program: &OsStr,
    action: &'static str,
    host: &Domain,
) -> Result<(), GhError> {
    let mut cmd = gh_command(program, true);
    cmd.args(["auth", action, "--hostname", host.as_str()]);
    let status = term.run(&mut cmd).map_err(|e| map_spawn(&e))?;
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

/// What [`Gh::repo`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    /// The repo, under its current owner and name.
    Found(Repo),
    /// HTTP 404: no such repo, or the viewer cannot see it.
    NotFound,
    /// HTTP 401 or 403; gh's message, one line.
    Denied(String),
}

/// A failure running `gh`.
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
        /// The exit status (gh printed its own message on the terminal).
        detail: String,
    },
    /// `gh api` exited non-zero or printed something unusable.
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
    use super::http_status;

    #[test]
    fn http_status_is_read_with_or_without_a_message() {
        assert_eq!(http_status("gh: Not Found (HTTP 404)"), Some(404));
        assert_eq!(http_status("gh: HTTP 404"), Some(404));
        assert_eq!(
            http_status(
                "gh: Resource protected by SAML enforcement. (HTTP 403) Authorize in your web browser: https://x"
            ),
            Some(403)
        );
        assert_eq!(
            http_status("gh: HTTP 502: Bad Gateway (https://x)"),
            Some(502)
        );
        assert_eq!(http_status("gh: HTTPS 404"), None);
        assert_eq!(http_status("gh: HTTP 4040"), None);
        assert_eq!(http_status("could not resolve host"), None);
    }
}
