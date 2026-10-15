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
    /// Extra variables that make tests hermetic (`GIT_CONFIG_GLOBAL`).
    #[cfg(test)]
    test_env: Vec<(OsString, OsString)>,
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
            #[cfg(test)]
            test_env: Vec::new(),
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
        #[cfg(test)]
        cmd.envs(self.test_env.iter().map(|(k, v)| (k, v)));
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

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::io::Write;
    use std::path::PathBuf;
    use std::process::Stdio;

    use super::*;
    use crate::core::sync_outcome::Upstream;

    /// Runs a fixture git command isolated from the user's config.
    fn git(dir: &Path, args: &[&str]) -> String {
        let out = proc::command("git")
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

    fn github() -> Domain {
        "github.com".parse().expect("domain")
    }

    /// A `ProcessGit` that ignores the developer's global and system config.
    fn process_git() -> ProcessGit {
        let mut pg = ProcessGit::new();
        pg.test_env = [
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ]
        .map(|(k, v)| (OsString::from(k), OsString::from(v)))
        .into();
        pg
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
            pg.clone_repo(&file_url(&origin), &local, &github())
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

        fn fetch(&self) {
            self.git.fetch(&self.local, &github()).expect("fetch");
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
    }

    #[test]
    fn fetch_then_fast_forward_catches_up() {
        let fx = Fixture::new();
        fx.push_upstream(&["b", "c"]);
        assert_eq!(fx.facts(), branch(false, 0, 0));
        fx.fetch();
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
        fx.fetch();
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
        fx.fetch();
        let merged = proc::command("git")
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
        fx.fetch();
        assert_eq!(fx.facts(), branch(false, 0, 1));
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
        let err = fx.git.fetch(&fx.local, &github()).unwrap_err();
        assert_matches!(&err, GitError::Failed { action: "fetch", detail } if !detail.is_empty());
        assert!(!err.to_string().contains('\n'), "{err}");
    }

    #[test]
    fn failed_clone_leaves_nothing_behind() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let url = file_url(&tmp.path().join("missing.git"));
        let dest = tmp.path().join("dest");
        let err = process_git()
            .clone_repo(&url, &dest, &github())
            .unwrap_err();
        assert_matches!(&err, GitError::Failed { action: "clone", detail } if !detail.is_empty());
        assert!(!err.to_string().contains('\n'), "{err}");
        assert!(!dest.exists());
    }

    #[test]
    fn clone_and_fetch_authenticate_through_gh_for_their_domain_only() {
        let host: Domain = "ghe.example.com".parse().expect("domain");
        let pg = process_git();
        let reset = ["-c", "credential.helper="];
        let helper = [
            "-c",
            "credential.https://ghe.example.com.helper=!gh auth git-credential",
        ];
        let args_of = |cmd: &Command| -> Vec<String> {
            cmd.get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect()
        };
        let clone = pg.command(None, Some(&host), &[OsStr::new("clone")]);
        let fetch = pg.command(Some(Path::new("/r")), Some(&host), &[OsStr::new("fetch")]);
        for cmd in [&clone, &fetch] {
            let args = args_of(cmd);
            let joined = args.join(" ");
            assert!(args.windows(2).any(|w| w == reset), "{args:?}");
            assert!(args.windows(2).any(|w| w == helper), "{args:?}");
            // The reset comes first, so it clears the user's helpers but
            // not ours.
            let at = |pair: &[&str]| args.windows(2).position(|w| w == pair);
            assert!(at(&reset) < at(&helper), "{args:?}");
            assert!(!joined.contains("github.com"), "{joined}");
            for needle in ["Authorization", "extraHeader", "extraheader"] {
                assert!(!joined.contains(needle), "{joined}");
                assert!(
                    cmd.get_envs().all(|(k, v)| {
                        !k.to_string_lossy().contains(needle)
                            && !v.is_some_and(|v| v.to_string_lossy().contains(needle))
                    }),
                    "{needle} in env"
                );
            }
            let envs: Vec<_> = cmd.get_envs().collect();
            for (name, value) in CHILD_ENV {
                assert!(envs.contains(&(OsStr::new(name), Some(OsStr::new(value)))));
            }
            assert!(envs.contains(&(OsStr::new("GIT_DIR"), None)));
        }
        let status = pg.command(Some(Path::new("/r")), None, &[OsStr::new("status")]);
        assert!(
            !args_of(&status).iter().any(|a| a.starts_with("credential")),
            "status needs no credentials"
        );
    }

    /// Runs `git credential fill` for `host` the way `fetch` would for
    /// `domain`, with a fake `gh` first on `PATH` and a user config whose
    /// own helper must never run. Returns git's stdout, or `None` when git
    /// found no credential.
    fn fill(tmp: &Path, domain: Option<&Domain>, host: &str) -> Option<String> {
        let mut cmd = ProcessGit::new().command(
            None,
            domain,
            &[OsStr::new("credential"), OsStr::new("fill")],
        );
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![tmp.join("bin")];
        paths.extend(std::env::split_paths(&path));
        cmd.env("PATH", std::env::join_paths(paths).expect("PATH"))
            .env("GIT_CONFIG_GLOBAL", tmp.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().expect("git runs");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(format!("protocol=https\nhost={host}\n\n").as_bytes())
            .expect("write");
        let out = child.wait_with_output().expect("git exits");
        out.status
            .success()
            .then(|| String::from_utf8(out.stdout).expect("utf8"))
    }

    #[test]
    fn real_git_asks_gh_for_the_scoped_host_and_ignores_user_helpers() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let bin = tmp.path().join("bin");
        std::fs::create_dir_all(&bin).expect("mkdir");
        let log = tmp.path().join("gh.log");
        let gh = bin.join("gh");
        std::fs::write(
            &gh,
            format!(
                "#!/bin/sh\necho \"$*\" >> '{}'\necho username=x-access-token\necho password=from-gh\n",
                log.display()
            ),
        )
        .expect("write gh");
        let mut perms = std::fs::metadata(&gh).expect("meta").permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(&gh, perms).expect("chmod");
        std::fs::write(
            tmp.path().join("gitconfig"),
            "[credential]\n\thelper = \"!f() { echo username=user; echo password=from-user; }; f\"\n\
             [credential \"https://ghe.example.com\"]\n\thelper = \"!f() { echo username=user; echo password=from-user; }; f\"\n",
        )
        .expect("write config");
        let domain: Domain = "ghe.example.com".parse().expect("domain");
        // Control: without the helper settings the user's helper answers.
        let control = fill(tmp.path(), None, "ghe.example.com").expect("user helper");
        assert!(control.contains("password=from-user"), "{control}");

        let got = fill(tmp.path(), Some(&domain), "ghe.example.com").expect("gh answers");
        assert!(got.contains("password=from-gh"), "{got}");
        assert!(!got.contains("from-user"), "{got}");
        assert_eq!(
            std::fs::read_to_string(&log).expect("gh ran"),
            "auth git-credential get\n"
        );

        // Another host gets neither gh nor the user's helper.
        assert_eq!(fill(tmp.path(), Some(&domain), "github.com"), None);
        assert_eq!(
            std::fs::read_to_string(&log).expect("log").lines().count(),
            1
        );
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
        let _ = proc::command("git")
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
        let mut cmd = process_git().command(None, Some(&github()), &args);
        // Environment config (`git -c` scope too) is overridden as well.
        cmd.env("GIT_CONFIG_GLOBAL", &config)
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "protocol.ext.allow")
            .env("GIT_CONFIG_VALUE_0", "always");
        let out = cmd.output().expect("runs");
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
        let _ = fx
            .git
            .command(None, Some(&github()), &args)
            .output()
            .expect("runs");
        assert!(
            !dest.join("sub").join("a").exists(),
            "file:// submodule was populated"
        );
    }

    #[test]
    fn missing_git_is_not_installed() {
        let mut pg = process_git();
        pg.program = OsString::from("work-no-such-git-w27");
        let err = pg.facts(Path::new("/")).unwrap_err();
        assert_eq!(err, GitError::NotInstalled);
    }
}
