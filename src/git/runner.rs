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

/// Set on every git child: git's messages are never localised, git never
/// prompts on the terminal, and `status` never takes `index.lock` under the
/// user's editor.
const CHILD_ENV: &[(&str, &str)] = &[
    ("LC_ALL", "C"),
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GIT_OPTIONAL_LOCKS", "0"),
];

/// The `-c` settings that make git authenticate through `gh` for `host`
/// only: an empty `credential.helper` clears every helper configured
/// before it (the user's, for this one command), and the URL-scoped helper
/// answers for `https://<host>` alone. git hands `-c` settings to its own
/// children, so submodules on the same host authenticate too.
fn credential_args(host: &Domain) -> [String; 4] {
    [
        "-c".to_owned(),
        "credential.helper=".to_owned(),
        "-c".to_owned(),
        format!("credential.https://{host}.helper=!gh auth git-credential"),
    ]
}

/// Runs `git` from `PATH`.
///
/// Every child gets `-c protocol.ext.allow=never` (which keeps `ext::`
/// blocked even if user config allows it; git's other protocol defaults are
/// left alone), the [`CHILD_ENV`] variables, and loses the [`REPO_VARS`] so
/// `-C <repo>` means what it says. Clone and fetch also get
/// [`credential_args`] for the repo's domain.
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

    /// A `git` command with the child environment applied; with `host`, it
    /// authenticates through `gh` for that domain.
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

    /// Runs git and returns its output whatever the exit status.
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

    /// Runs git and returns its stdout, or [`GitError::Failed`].
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

/// Maps a spawn error: not found means git is not installed.
fn spawn_error(err: &io::Error) -> GitError {
    if err.kind() == io::ErrorKind::NotFound {
        GitError::NotInstalled
    } else {
        GitError::Spawn {
            detail: text::one_line(&err.to_string()),
        }
    }
}

/// A failed run, with git's stderr reduced to one line.
fn failed(action: &'static str, out: &Output) -> GitError {
    let line = proc::stderr_summary(&out.stderr);
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
