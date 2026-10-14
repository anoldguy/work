//! [`ProcessGit`]: the real git, run as a child process.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::OnceLock;

use crate::auth::GitAuth;
use crate::core::sync_outcome::RepoFacts;
use crate::domain::CloneUrl;
use crate::env::{EnvSnapshot, GitConfigCount};
use crate::git::auth_env::git_env;
use crate::git::{Git, GitError, status};
use crate::proc;

/// The oldest git that reads config from `GIT_CONFIG_COUNT` (2.31).
const MIN_VERSION: (u32, u32) = (2, 31);

/// Exit code of `git remote get-url` for a missing remote.
const NO_SUCH_REMOTE: i32 = 2;

/// Runs `git` from `PATH` with the environment from `git::auth_env`.
///
/// The first call checks `git --version` (2.31 or later) once per process.
#[derive(Debug)]
pub struct ProcessGit {
    config_count: GitConfigCount,
    trace_vars: Vec<OsString>,
    program: OsString,
    version: OnceLock<Result<(), GitError>>,
    /// Extra variables that make tests hermetic (`GIT_CONFIG_GLOBAL`).
    #[cfg(test)]
    test_env: Vec<(OsString, OsString)>,
}

impl ProcessGit {
    /// Copies what it needs from `env` (`GIT_CONFIG_COUNT` and the trace
    /// variable names to strip).
    #[must_use]
    pub fn new(env: &EnvSnapshot) -> Self {
        Self {
            config_count: env.git_config_count,
            trace_vars: env.git_trace_vars.clone(),
            program: OsString::from("git"),
            version: OnceLock::new(),
            #[cfg(test)]
            test_env: Vec::new(),
        }
    }

    /// A `git` command with the child environment applied. Callers never
    /// format it with `{:?}`.
    fn command(&self, repo: Option<&Path>, args: &[&OsStr], auth: Option<&GitAuth<'_>>) -> Command {
        let mut cmd = Command::new(&self.program);
        if let Some(repo) = repo {
            cmd.arg("-C").arg(repo);
        }
        cmd.args(args);
        git_env(auth, self.config_count, &self.trace_vars).apply(&mut cmd);
        #[cfg(test)]
        cmd.envs(self.test_env.iter().map(|(k, v)| (k, v)));
        cmd
    }

    /// Checks the git version once; later calls repeat the first answer.
    fn ensure_version(&self) -> Result<(), GitError> {
        self.version
            .get_or_init(|| {
                let out =
                    proc::run_capture(&mut self.command(None, &[OsStr::new("--version")], None))
                        .map_err(|e| spawn_error(&e))?;
                if !out.status.success() {
                    return Err(failed("version", &out, None));
                }
                check_version(&String::from_utf8_lossy(&out.stdout))
            })
            .clone()
    }

    /// Runs git and returns its output whatever the exit status.
    fn run_raw(
        &self,
        repo: Option<&Path>,
        args: &[&OsStr],
        auth: Option<&GitAuth<'_>>,
    ) -> Result<Output, GitError> {
        self.ensure_version()?;
        proc::run_capture(&mut self.command(repo, args, auth)).map_err(|e| spawn_error(&e))
    }

    /// Runs git and returns its stdout, or [`GitError::Failed`].
    fn run(
        &self,
        action: &'static str,
        repo: Option<&Path>,
        args: &[&str],
        auth: Option<&GitAuth<'_>>,
    ) -> Result<Vec<u8>, GitError> {
        let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
        let out = self.run_raw(repo, &args, auth)?;
        if out.status.success() {
            Ok(out.stdout)
        } else {
            Err(failed(action, &out, auth))
        }
    }
}

/// Maps a spawn error: not found means git is not installed.
fn spawn_error(err: &io::Error) -> GitError {
    if err.kind() == io::ErrorKind::NotFound {
        GitError::NotInstalled
    } else {
        GitError::Spawn {
            detail: proc::one_line(err.to_string().as_bytes()),
        }
    }
}

/// Added when an authenticated request was refused a redirect
/// (`http.<url>.followRedirects=false` keeps the token on its host).
const REDIRECT_HINT: &str =
    "(the repository moved; run 'work update' and re-clone, or 'git remote set-url')";

/// True when git's one-line error is an HTTP 3xx status.
fn is_redirect_error(line: &str) -> bool {
    line.split_once("returned error: 3").is_some()
}

/// A failed run: stderr reduced to one line and redacted, with a hint when
/// an authenticated request hit a redirect.
fn failed(action: &'static str, out: &Output, auth: Option<&GitAuth<'_>>) -> GitError {
    let line = proc::one_line(&out.stderr);
    let line = if line.is_empty() {
        out.status.to_string()
    } else if auth.is_some() && is_redirect_error(&line) {
        format!("{line} {REDIRECT_HINT}")
    } else {
        line
    };
    GitError::Failed {
        action,
        detail: match auth {
            Some(auth) => auth.redact(&line),
            None => line,
        },
    }
}

/// Checks `git --version` output against [`MIN_VERSION`].
fn check_version(stdout: &str) -> Result<(), GitError> {
    let version = stdout
        .trim()
        .strip_prefix("git version ")
        .and_then(|rest| rest.split_whitespace().next());
    let Some(version) = version else {
        return Err(GitError::Failed {
            action: "version",
            detail: "unrecognised `git --version` output".to_owned(),
        });
    };
    // Leading digits of each part, so `2.31-rc0` reads as 2.31.
    let number = |part: &str| {
        let end = part
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(part.len());
        part[..end].parse::<u32>().ok()
    };
    let mut parts = version.split('.').map(number);
    match (parts.next(), parts.next()) {
        (Some(Some(major)), Some(Some(minor))) if (major, minor) >= MIN_VERSION => Ok(()),
        _ => Err(GitError::TooOld {
            version: version.to_owned(),
        }),
    }
}

impl Git for ProcessGit {
    fn clone_repo(
        &self,
        url: &CloneUrl,
        dest: &Path,
        auth: Option<&GitAuth<'_>>,
    ) -> Result<(), GitError> {
        let existed = dest.exists();
        let args = [
            OsStr::new("clone"),
            OsStr::new("--quiet"),
            OsStr::new("--"),
            OsStr::new(url.as_str()),
            dest.as_os_str(),
        ];
        let out = self.run_raw(None, &args, auth)?;
        if out.status.success() {
            return Ok(());
        }
        if !existed && dest.exists() {
            // git normally removes its own partial clone; this is a backstop.
            let _ = std::fs::remove_dir_all(dest);
        }
        Err(failed("clone", &out, auth))
    }

    fn fetch(&self, repo: &Path, auth: Option<&GitAuth<'_>>) -> Result<(), GitError> {
        self.run("fetch", Some(repo), &["fetch", "--quiet"], auth)
            .map(drop)
    }

    fn facts(&self, repo: &Path) -> Result<RepoFacts, GitError> {
        let stdout = self.run(
            "status",
            Some(repo),
            &[
                "status",
                "--porcelain=v2",
                "--branch",
                "--untracked-files=normal",
            ],
            None,
        )?;
        status::parse_v2(&String::from_utf8_lossy(&stdout))
    }

    fn fast_forward(&self, repo: &Path) -> Result<(), GitError> {
        self.run(
            "merge",
            Some(repo),
            &["merge", "--ff-only", "--quiet", "@{u}"],
            None,
        )
        .map(drop)
    }

    fn origin_url(&self, repo: &Path) -> Result<Option<String>, GitError> {
        let args = ["remote", "get-url", "origin"].map(OsStr::new);
        let out = self.run_raw(Some(repo), &args, None)?;
        if out.status.success() {
            let url = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            return Ok((!url.is_empty()).then_some(url));
        }
        if out.status.code() == Some(NO_SUCH_REMOTE)
            && String::from_utf8_lossy(&out.stderr).contains("No such remote")
        {
            return Ok(None);
        }
        Err(failed("remote", &out, None))
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::path::PathBuf;

    use super::*;
    use crate::auth::SecretToken;
    use crate::core::sync_outcome::Upstream;
    use crate::domain::Domain;

    const TOKEN: &str = "gho_runnersecret123";

    /// Runs a fixture git command isolated from the user's config.
    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8(out.stdout).expect("utf8")
    }

    fn commit(dir: &Path, file: &str) {
        std::fs::write(dir.join(file), file).expect("write");
        git(dir, &["add", file]);
        git(dir, &["commit", "-q", "-m", file]);
    }

    fn file_url(path: &Path) -> CloneUrl {
        CloneUrl::try_from(format!("file://{}", path.display())).expect("valid clone url")
    }

    /// A `ProcessGit` that ignores the developer's global and system config.
    fn hermetic(env: &EnvSnapshot) -> ProcessGit {
        let mut pg = ProcessGit::new(env);
        pg.test_env = [
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ]
        .map(|(k, v)| (OsString::from(k), OsString::from(v)))
        .into();
        pg
    }

    fn process_git() -> ProcessGit {
        hermetic(&EnvSnapshot::default())
    }

    /// A bare origin with one commit, a seed clone that pushes to it, and a
    /// `local` clone made by `ProcessGit`.
    struct Fixture {
        _tmp: tempfile::TempDir,
        origin: PathBuf,
        seed: PathBuf,
        local: PathBuf,
        git: ProcessGit,
    }

    impl Fixture {
        fn new() -> Self {
            let tmp = tempfile::tempdir().expect("tempdir");
            let origin = tmp.path().join("origin.git");
            let seed = tmp.path().join("seed");
            let local = tmp.path().join("work").join("local");
            git(
                tmp.path(),
                &["init", "-q", "--bare", "-b", "main", "origin.git"],
            );
            git(tmp.path(), &["clone", "-q", "origin.git", "seed"]);
            git(&seed, &["checkout", "-q", "-b", "main"]);
            commit(&seed, "a");
            git(&seed, &["push", "-q", "-u", "origin", "main"]);
            std::fs::create_dir_all(local.parent().expect("parent")).expect("mkdir");
            let pg = process_git();
            pg.clone_repo(&file_url(&origin), &local, None)
                .expect("clone works");
            Self {
                _tmp: tmp,
                origin,
                seed,
                local,
                git: pg,
            }
        }

        fn push_upstream(&self, files: &[&str]) {
            for f in files {
                commit(&self.seed, f);
            }
            git(&self.seed, &["push", "-q", "origin", "main"]);
        }

        fn facts(&self) -> RepoFacts {
            self.git.facts(&self.local).expect("facts")
        }
    }

    fn branch(dirty: bool, ahead: u32, behind: u32) -> RepoFacts {
        RepoFacts::Branch {
            dirty,
            upstream: Upstream::Tracking { ahead, behind },
        }
    }

    #[test]
    fn fresh_clone_is_clean_and_up_to_date() {
        let fx = Fixture::new();
        assert!(fx.local.join(".git").is_dir());
        assert_eq!(fx.facts(), branch(false, 0, 0));
        assert_eq!(
            fx.git.origin_url(&fx.local).expect("origin"),
            Some(file_url(&fx.origin).as_str().to_owned())
        );
    }

    #[test]
    fn fetch_then_fast_forward_catches_up() {
        let fx = Fixture::new();
        fx.push_upstream(&["b", "c"]);
        assert_eq!(fx.facts(), branch(false, 0, 0));
        fx.git.fetch(&fx.local, None).expect("fetch");
        assert_eq!(fx.facts(), branch(false, 0, 2));
        fx.git.fast_forward(&fx.local).expect("ff");
        assert_eq!(fx.facts(), branch(false, 0, 0));
        assert!(fx.local.join("c").exists());
    }

    #[test]
    fn ahead_and_diverged_states() {
        let fx = Fixture::new();
        commit(&fx.local, "mine");
        assert_eq!(fx.facts(), branch(false, 1, 0));
        fx.push_upstream(&["theirs"]);
        fx.git.fetch(&fx.local, None).expect("fetch");
        assert_eq!(fx.facts(), branch(false, 1, 1));
        let err = fx.git.fast_forward(&fx.local).unwrap_err();
        assert_matches!(&err, GitError::Failed { action: "merge", detail } if !detail.is_empty());
        assert!(!err.to_string().contains('\n'), "{err}");
    }

    #[test]
    fn untracked_and_modified_files_are_dirty() {
        let fx = Fixture::new();
        std::fs::write(fx.local.join("new"), "x").expect("write");
        assert_eq!(fx.facts(), branch(true, 0, 0));
        std::fs::remove_file(fx.local.join("new")).expect("rm");
        std::fs::write(fx.local.join("a"), "changed").expect("write");
        assert_eq!(fx.facts(), branch(true, 0, 0));
    }

    #[test]
    fn untracked_files_count_even_when_config_hides_them() {
        let fx = Fixture::new();
        git(&fx.local, &["config", "status.showUntrackedFiles", "no"]);
        std::fs::write(fx.local.join("new"), "x").expect("write");
        assert_eq!(fx.facts(), branch(true, 0, 0));
    }

    #[test]
    fn staged_only_change_is_dirty_and_ignored_file_is_not() {
        let fx = Fixture::new();
        std::fs::write(fx.local.join(".git/info/exclude"), "*.log\n").expect("write");
        std::fs::write(fx.local.join("build.log"), "x").expect("write");
        assert_eq!(fx.facts(), branch(false, 0, 0));
        std::fs::write(fx.local.join("a"), "staged").expect("write");
        git(&fx.local, &["add", "a"]);
        assert_eq!(fx.facts(), branch(true, 0, 0));
    }

    #[test]
    fn merge_conflict_is_dirty() {
        let fx = Fixture::new();
        std::fs::write(fx.local.join("a"), "mine").expect("write");
        git(&fx.local, &["commit", "-q", "-am", "mine"]);
        std::fs::write(fx.seed.join("a"), "theirs").expect("write");
        git(&fx.seed, &["commit", "-q", "-am", "theirs"]);
        git(&fx.seed, &["push", "-q", "origin", "main"]);
        fx.git.fetch(&fx.local, None).expect("fetch");
        let merged = Command::new("git")
            .args(["merge", "-q", "@{u}"])
            .current_dir(&fx.local)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
            .output()
            .expect("git runs");
        assert!(!merged.status.success(), "the merge conflicts");
        assert_eq!(fx.facts(), branch(true, 1, 1));
    }

    #[test]
    fn unborn_branch_has_no_upstream() {
        let tmp = tempfile::tempdir().expect("tempdir");
        git(tmp.path(), &["init", "-q", "-b", "main", "."]);
        assert_eq!(
            process_git().facts(tmp.path()).expect("facts"),
            RepoFacts::Branch {
                dirty: false,
                upstream: Upstream::None
            }
        );
    }

    #[test]
    fn detached_head_clean_and_dirty() {
        let fx = Fixture::new();
        git(&fx.local, &["checkout", "-q", "--detach"]);
        assert_eq!(fx.facts(), RepoFacts::Detached { dirty: false });
        std::fs::write(fx.local.join("new"), "x").expect("write");
        assert_eq!(fx.facts(), RepoFacts::Detached { dirty: true });
    }

    #[test]
    fn local_only_branch_has_no_upstream() {
        let fx = Fixture::new();
        git(&fx.local, &["checkout", "-q", "-b", "topic"]);
        assert_eq!(
            fx.facts(),
            RepoFacts::Branch {
                dirty: false,
                upstream: Upstream::None
            }
        );
        assert_matches!(
            fx.git.fast_forward(&fx.local),
            Err(GitError::Failed {
                action: "merge",
                ..
            })
        );
    }

    #[test]
    fn fetch_uses_the_branch_remote_not_origin() {
        let fx = Fixture::new();
        git(&fx.local, &["remote", "rename", "origin", "upstream"]);
        fx.push_upstream(&["b"]);
        fx.git
            .fetch(&fx.local, None)
            .expect("fetch via branch remote");
        assert_eq!(fx.facts(), branch(false, 0, 1));
        assert_eq!(fx.git.origin_url(&fx.local).expect("no error"), None);
    }

    #[test]
    fn missing_origin_is_none_and_other_failures_are_errors() {
        let fx = Fixture::new();
        git(&fx.local, &["remote", "remove", "origin"]);
        assert_eq!(fx.git.origin_url(&fx.local).expect("no error"), None);
        let not_repo = fx.local.join("no-such-dir");
        assert_matches!(
            fx.git.origin_url(&not_repo),
            Err(GitError::Failed {
                action: "remote",
                ..
            })
        );
        assert_matches!(
            fx.git.facts(&not_repo),
            Err(GitError::Failed {
                action: "status",
                ..
            })
        );
    }

    #[test]
    fn fetch_failure_is_one_line() {
        let fx = Fixture::new();
        git(
            &fx.local,
            &[
                "remote",
                "set-url",
                "origin",
                "file:///nonexistent/w27/repo.git",
            ],
        );
        let err = fx.git.fetch(&fx.local, None).unwrap_err();
        assert_matches!(&err, GitError::Failed { action: "fetch", detail } if !detail.is_empty());
        assert!(!err.to_string().contains('\n'), "{err}");
    }

    #[test]
    fn failed_clone_leaves_nothing_behind() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let url = file_url(&tmp.path().join("missing.git"));
        let dest = tmp.path().join("dest");
        let err = process_git().clone_repo(&url, &dest, None).unwrap_err();
        assert_matches!(&err, GitError::Failed { action: "clone", detail } if !detail.is_empty());
        assert!(!err.to_string().contains('\n'), "{err}");
        assert!(!dest.exists());
    }

    #[test]
    fn clone_stderr_is_redacted() {
        let fx = Fixture::new();
        let token = SecretToken::new(TOKEN).expect("token");
        let auth = GitAuth::new("github.com".parse::<Domain>().expect("domain"), &token);
        // git names the occupied destination in its one-line error.
        let dest = fx.local.with_file_name(TOKEN);
        std::fs::create_dir_all(dest.join("occupied")).expect("mkdir");
        let err = fx
            .git
            .clone_repo(&file_url(&fx.origin), &dest, Some(&auth))
            .unwrap_err();
        let shown = err.to_string();
        assert_matches!(
            &err,
            GitError::Failed {
                action: "clone",
                ..
            }
        );
        assert!(shown.contains("***"), "{shown}");
        assert!(!shown.contains(TOKEN), "{shown}");
        assert!(dest.join("occupied").exists(), "pre-existing dest kept");
    }

    #[test]
    fn ext_transport_is_blocked_even_when_user_config_allows_it() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = tmp.path().join("gitconfig");
        std::fs::write(&config, "[protocol \"ext\"]\n\tallow = always\n").expect("write");
        let ext_cmd = |marker: &Path| {
            let ext = format!("ext::sh -c touch% {}", marker.display());
            [OsString::from("ls-remote"), OsString::from(ext)]
        };
        // Control: the user's config alone lets the ext command run.
        let control = tmp.path().join("control");
        let _ = Command::new("git")
            .args(ext_cmd(&control))
            .env("GIT_CONFIG_GLOBAL", &config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_ALLOW_PROTOCOL")
            .output()
            .expect("git runs");
        assert!(control.exists(), "control: ext runs under the user config");

        let marker = tmp.path().join("pwned");
        let args = ext_cmd(&marker);
        let args: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let mut cmd = process_git().command(None, &args, None);
        cmd.env("GIT_CONFIG_GLOBAL", &config);
        let out = proc::run_capture(&mut cmd).expect("runs");
        assert!(!out.status.success());
        assert!(!marker.exists());
    }

    #[test]
    fn file_submodules_stay_blocked() {
        let fx = Fixture::new();
        let tmp = fx.origin.parent().expect("parent");
        git(tmp, &["init", "-q", "-b", "main", "super"]);
        let sup = tmp.join("super");
        git(
            &sup,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-q",
                file_url(&fx.origin).as_str(),
                "sub",
            ],
        );
        git(&sup, &["commit", "-q", "-m", "add sub"]);
        let dest = tmp.join("superclone");
        let url = file_url(&sup);
        let args = [
            OsStr::new("clone"),
            OsStr::new("-q"),
            OsStr::new("--recurse-submodules"),
            OsStr::new(url.as_str()),
            dest.as_os_str(),
        ];
        let _ = proc::run_capture(&mut fx.git.command(None, &args, None)).expect("runs");
        assert!(
            !dest.join("sub").join("a").exists(),
            "file:// submodule was populated"
        );
    }

    #[test]
    fn argv_never_contains_the_token() {
        let token = SecretToken::new(TOKEN).expect("token");
        let auth = GitAuth::new("github.com".parse::<Domain>().expect("domain"), &token);
        let pg = process_git();
        let url = CloneUrl::try_from("https://github.com/acme/foo.git".to_owned()).expect("url");
        let args = [
            OsStr::new("clone"),
            OsStr::new(url.as_str()),
            OsStr::new("/tmp/x"),
        ];
        let cmd = pg.command(Some(Path::new("/tmp")), &args, Some(&auth));
        let b64 = auth.header_b64().expose();
        for arg in cmd.get_args() {
            let arg = arg.to_string_lossy();
            assert!(!arg.contains(TOKEN) && !arg.contains(b64), "{arg}");
        }
        let header = format!("Authorization: Basic {b64}");
        assert!(
            cmd.get_envs()
                .any(|(k, v)| k == "GIT_CONFIG_VALUE_2" && v == Some(OsStr::new(&header)))
        );
    }

    #[test]
    fn real_git_reads_the_header_after_existing_config_entries() {
        let token = SecretToken::new(TOKEN).expect("token");
        let auth = GitAuth::new("ghe.example.com".parse::<Domain>().expect("domain"), &token);
        let env = EnvSnapshot {
            git_config_count: GitConfigCount::Count(1),
            ..EnvSnapshot::default()
        };
        let pg = hermetic(&env);
        let get = |key: &str| {
            let args = [OsStr::new("config"), OsStr::new("--get"), OsStr::new(key)];
            let mut cmd = pg.command(None, &args, Some(&auth));
            cmd.env("GIT_CONFIG_KEY_0", "user.name")
                .env("GIT_CONFIG_VALUE_0", "Existing");
            let out = proc::run_capture(&mut cmd).expect("runs");
            assert!(out.status.success(), "{key}");
            String::from_utf8(out.stdout)
                .expect("utf8")
                .trim()
                .to_owned()
        };
        assert_eq!(get("user.name"), "Existing");
        assert_eq!(
            get("http.https://ghe.example.com/.extraheader"),
            format!("Authorization: Basic {}", auth.header_b64().expose())
        );
    }

    #[test]
    fn trace_variables_are_removed_and_locale_forced() {
        let env = EnvSnapshot {
            git_trace_vars: vec![OsString::from("GIT_TRACE_CUSTOM")],
            ..EnvSnapshot::default()
        };
        let cmd = ProcessGit::new(&env).command(None, &[], None);
        let envs: Vec<_> = cmd.get_envs().collect();
        for name in ["GIT_TRACE_CUSTOM", "GIT_TRACE", "GIT_CURL_VERBOSE"] {
            assert!(envs.contains(&(OsStr::new(name), None)), "{name}");
        }
        assert!(envs.contains(&(OsStr::new("LC_ALL"), Some(OsStr::new("C")))));
        assert!(envs.contains(&(OsStr::new("GIT_TERMINAL_PROMPT"), Some(OsStr::new("0")))));
        assert!(envs.contains(&(OsStr::new("GIT_OPTIONAL_LOCKS"), Some(OsStr::new("0")))));
        assert!(!envs.iter().any(|(k, _)| *k == "GIT_ALLOW_PROTOCOL"));
    }

    #[test]
    fn redirect_failure_gets_a_hint_only_with_auth() {
        let token = SecretToken::new(TOKEN).expect("token");
        let auth = GitAuth::new("github.com".parse::<Domain>().expect("domain"), &token);
        let out = Command::new("sh")
            .args([
                "-c",
                "echo \"fatal: unable to access 'https://github.com/a/b/': The requested URL returned error: 301\" >&2; exit 128",
            ])
            .output()
            .expect("sh runs");
        let with = failed("fetch", &out, Some(&auth)).to_string();
        assert!(with.ends_with(REDIRECT_HINT), "{with}");
        let without = failed("fetch", &out, None).to_string();
        assert!(!without.contains(REDIRECT_HINT), "{without}");
        assert!(!is_redirect_error("returned error: 404"));
    }

    #[test]
    fn version_gate() {
        assert_matches!(check_version("git version 2.31.0\n"), Ok(()));
        assert_matches!(check_version("git version 2.39.3 (Apple Git-145)"), Ok(()));
        assert_matches!(check_version("git version 2.45.1.windows.1"), Ok(()));
        assert_matches!(check_version("git version 3.0"), Ok(()));
        assert_matches!(check_version("git version 2.31-rc0"), Ok(()));
        assert_matches!(check_version("git version 2.31.0.rc1"), Ok(()));
        let err = check_version("git version 2.30.9").unwrap_err();
        assert_matches!(&err, GitError::TooOld { version } if version == "2.30.9");
        assert_eq!(
            err.to_string(),
            "git 2.30.9 is too old (2.31 or later is required)"
        );
        assert_matches!(
            check_version("git version 1.9"),
            Err(GitError::TooOld { .. })
        );
        assert_matches!(check_version("nonsense"), Err(GitError::Failed { .. }));
        assert_matches!(process_git().ensure_version(), Ok(()));
    }

    #[test]
    fn missing_git_is_not_installed() {
        let mut pg = process_git();
        pg.program = OsString::from("work-no-such-git-w27");
        let err = pg.facts(Path::new("/")).unwrap_err();
        assert_eq!(err, GitError::NotInstalled);
    }
}
