//! Git access through the `git` CLI, and parsers for its output.

pub mod status;

pub use status::{RepoFacts, Upstream};

use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::process::{Command, Output};

use crate::domain::{CloneUrl, Domain};
use crate::proc;
use crate::text;

const GIT: &str = "git";

/// Variables that would point git at a different repository than `-C`.
const REPO_VARS: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
];

/// `GIT_OPTIONAL_LOCKS=0` keeps `status` from taking `index.lock` under the user's editor.
const CHILD_ENV: &[(&str, &str)] = &[
    ("LC_ALL", "C"),
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GIT_OPTIONAL_LOCKS", "0"),
];

/// `git clone <url> <dest>`, authenticating through `gh` for `host`;
/// `dest`'s parent already exists.
///
/// # Errors
///
/// Any [`GitError`]; a failed clone leaves no partial `dest` behind.
pub fn clone_repo(url: &CloneUrl, dest: &Path, host: &Domain) -> Result<(), GitError> {
    let existed = dest.exists();
    let args = [
        OsStr::new("clone"),
        OsStr::new("--quiet"),
        OsStr::new("--"),
        OsStr::new(url.as_str()),
        dest.as_os_str(),
    ];
    let out = run_raw(None, Some(host), &args)?;
    if out.status.success() {
        return Ok(());
    }
    if !existed && dest.exists() {
        // git normally removes its own partial clone; this is a backstop.
        let _ = std::fs::remove_dir_all(dest);
    }
    Err(failed("clone", &out))
}

/// `git fetch --quiet` in `repo`, authenticating through `gh` for `host`.
///
/// # Errors
///
/// Any [`GitError`].
pub fn fetch(repo: &Path, host: &Domain) -> Result<(), GitError> {
    run("fetch", repo, Some(host), &["fetch", "--quiet"]).map(drop)
}

/// The branch, upstream and dirty state of `repo`.
///
/// # Errors
///
/// Any [`GitError`], including [`GitError::Status`] for unparsable
/// output.
pub fn facts(repo: &Path) -> Result<RepoFacts, GitError> {
    let stdout = run(
        "status",
        repo,
        None,
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=normal",
        ],
    )?;
    status::parse_v2(&String::from_utf8_lossy(&stdout))
}

/// `git merge --ff-only --quiet @{u}`.
///
/// # Errors
///
/// Any [`GitError`].
pub fn fast_forward(repo: &Path) -> Result<(), GitError> {
    run(
        "merge",
        repo,
        None,
        &["merge", "--ff-only", "--quiet", "@{u}"],
    )
    .map(drop)
}

/// An empty `credential.helper` clears the user's helpers for this command,
/// so only the URL-scoped `gh` helper answers, and only for `https://<host>`.
fn credential_args(host: &Domain) -> [String; 4] {
    [
        "-c".to_owned(),
        "credential.helper=".to_owned(),
        "-c".to_owned(),
        format!("credential.https://{host}.helper=!gh auth git-credential"),
    ]
}

/// `protocol.ext.allow=never` keeps `ext::` blocked even if user config allows it.
fn git_command(repo: Option<&Path>, host: Option<&Domain>, args: &[&OsStr]) -> Command {
    let mut cmd = proc::command(GIT);
    cmd.args(["-c", "protocol.ext.allow=never"]);
    if let Some(host) = host {
        cmd.args(credential_args(host));
    }
    if let Some(repo) = repo {
        cmd.arg("-C").arg(repo);
    }
    cmd.args(args);
    for name in REPO_VARS {
        cmd.env_remove(name);
    }
    cmd.envs(CHILD_ENV.iter().copied());
    cmd
}

fn run_raw(
    repo: Option<&Path>,
    host: Option<&Domain>,
    args: &[&OsStr],
) -> Result<Output, GitError> {
    git_command(repo, host, args)
        .output()
        .map_err(|e| spawn_error(&e))
}

fn run(
    action: &'static str,
    repo: &Path,
    host: Option<&Domain>,
    args: &[&str],
) -> Result<Vec<u8>, GitError> {
    let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    let out = run_raw(Some(repo), host, &args)?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(failed(action, &out))
    }
}

fn spawn_error(err: &io::Error) -> GitError {
    if err.kind() == io::ErrorKind::NotFound {
        GitError::NotInstalled
    } else {
        GitError::Spawn {
            detail: text::one_line(&err.to_string()),
        }
    }
}

fn failed(action: &'static str, out: &Output) -> GitError {
    let line = proc::stderr_summary(&out.stderr, proc::Fallback::LastLine);
    GitError::Failed {
        action,
        detail: if line.is_empty() {
            out.status.to_string()
        } else {
            line
        },
    }
}

/// A failure running git. Every `detail` is one line.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GitError {
    /// `git` is not on `PATH`.
    #[error("git is required (install git and make sure it is on PATH)")]
    NotInstalled,
    /// git could not be started for another reason.
    #[error("could not run git: {detail}")]
    Spawn {
        /// The OS error, one line.
        detail: String,
    },
    /// `git <action>` ran and failed.
    #[error("git {action} failed: {detail}")]
    Failed {
        /// `clone`, `fetch`, `status`, or `merge`.
        action: &'static str,
        /// git's stderr, reduced to one line.
        detail: String,
    },
    /// `git status` output did not have the expected shape.
    #[error("unexpected git status output ({detail})")]
    Status {
        /// What was unexpected.
        detail: String,
    },
}
