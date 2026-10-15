//! [`ProcessGit`]: the real git, run as a child process.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::process::{Command, Output};

use crate::core::sync_outcome::RepoFacts;
use crate::domain::{CloneUrl, Domain};
use crate::git::{Git, GitError, status};
use crate::proc;
use crate::text;

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

/// Runs `git` from `PATH`.
#[derive(Debug)]
pub struct ProcessGit {
    program: OsString,
}

impl Default for ProcessGit {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessGit {
    /// The `git` on `PATH`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            program: OsString::from("git"),
        }
    }

    /// `protocol.ext.allow=never` keeps `ext::` blocked even if user config allows it.
    fn command(&self, repo: Option<&Path>, host: Option<&Domain>, args: &[&OsStr]) -> Command {
        let mut cmd = proc::command(&self.program);
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
        &self,
        repo: Option<&Path>,
        host: Option<&Domain>,
        args: &[&OsStr],
    ) -> Result<Output, GitError> {
        self.command(repo, host, args)
            .output()
            .map_err(|e| spawn_error(&e))
    }

    fn run(
        &self,
        action: &'static str,
        repo: &Path,
        host: Option<&Domain>,
        args: &[&str],
    ) -> Result<Vec<u8>, GitError> {
        let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
        let out = self.run_raw(Some(repo), host, &args)?;
        if out.status.success() {
            Ok(out.stdout)
        } else {
            Err(failed(action, &out))
        }
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

impl Git for ProcessGit {
    fn clone_repo(&self, url: &CloneUrl, dest: &Path, host: &Domain) -> Result<(), GitError> {
        let existed = dest.exists();
        let args = [
            OsStr::new("clone"),
            OsStr::new("--quiet"),
            OsStr::new("--"),
            OsStr::new(url.as_str()),
            dest.as_os_str(),
        ];
        let out = self.run_raw(None, Some(host), &args)?;
        if out.status.success() {
            return Ok(());
        }
        if !existed && dest.exists() {
            // git normally removes its own partial clone; this is a backstop.
            let _ = std::fs::remove_dir_all(dest);
        }
        Err(failed("clone", &out))
    }

    fn fetch(&self, repo: &Path, host: &Domain) -> Result<(), GitError> {
        self.run("fetch", repo, Some(host), &["fetch", "--quiet"])
            .map(drop)
    }

    fn facts(&self, repo: &Path) -> Result<RepoFacts, GitError> {
        let stdout = self.run(
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

    fn fast_forward(&self, repo: &Path) -> Result<(), GitError> {
        self.run(
            "merge",
            repo,
            None,
            &["merge", "--ff-only", "--quiet", "@{u}"],
        )
        .map(drop)
    }
}
