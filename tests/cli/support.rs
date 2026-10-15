//! A hermetic home for one test: real `git`, a fake `gh` on `PATH`, and
//! logging editor and opener scripts.
//!
//! ```text
//! <root>/home          HOME, the command's cwd; work_dir is home/code
//! <root>/bin           the whole PATH: gh, git, editor, the opener, sh, work
//! <root>/fixtures      gh api answers: <host>/<path>.jsonl, or <path>.err
//! <root>/remotes       bare repos; https://<host>/ is rewritten to remotes/<host>/
//! <root>/<name>.log    one line per run of gh, git, editor or the opener
//! ```

use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;
use tempfile::TempDir;

/// Every `api` call prints `fixtures/<host>/<path>.jsonl` (the path without
/// its query), or fails with `<path>.err` on stderr. `auth status|login|
/// logout --hostname <host>` exits with the code in `auth-<host>`, if any.
const FAKE_GH: &str = r#"#!/bin/sh
root=${0%/*}/..
printf '%s\n' "$*" >> "$root/gh.log" || exit 1
emit() { while IFS= read -r l || [ -n "$l" ]; do printf '%s\n' "$l"; done < "$1"; }
case "$1 $2" in
  "api --hostname")
    host=$3
    shift 3
    if [ "$1" = --paginate ]; then shift; fi
    fixture="$root/fixtures/$host/${1%%\?*}"
    if [ -f "$fixture.err" ]; then emit "$fixture.err" >&2; exit 1; fi
    if [ -f "$fixture.jsonl" ]; then emit "$fixture.jsonl"; exit 0; fi
    printf 'gh: Not Found (HTTP 404)\n' >&2
    exit 1 ;;
  "auth git-credential")
    if [ "$3" = get ]; then printf 'username=x-access-token\npassword=fake\n'; fi
    exit 0 ;;
  "auth status"|"auth login"|"auth logout")
    code=0
    if [ -f "$root/auth-$4" ]; then read -r code < "$root/auth-$4"; fi
    if [ "$code" = 0 ]; then printf 'Logged in to %s\n' "$4"; fi
    exit "$code" ;;
esac
printf 'fake gh: unsupported: %s\n' "$*" >&2
exit 1
"#;

/// Records its pid, then its physical cwd and argv, in `<root>/<own name>.pid`
/// and `.log`. Each write is one append that exits if the directory is gone.
const LOGGER: &str = r#"#!/bin/sh
log="${0%/*}/../${0##*/}"
printf '%s\n' "$$" >> "$log.pid" || exit 0
printf '%s\t%s\n' "$(pwd -P)" "$*" >> "$log.log" || exit 0
"#;

const OPENER: &str = if cfg!(target_os = "macos") {
    "open"
} else {
    "xdg-open"
};

/// The configured `work_dir`, relative to `HOME`.
const WORK_DIR: &str = "code";

/// A cache timestamp older than any staleness window.
const STALE: &str = "2000-01-01T00:00:00Z";

pub struct TestEnv {
    root: TempDir,
}

impl TestEnv {
    /// A home with an empty `work_dir`, no config, no cache and no fixtures.
    pub fn new() -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        let env = Self { root };
        for dir in ["home/code", "bin", "fixtures", "remotes"] {
            fs::create_dir_all(env.root().join(dir)).expect("create dirs");
        }
        let bin = env.root().join("bin");
        let templates = templates();
        for (name, template) in [
            ("gh", &templates.gh),
            ("git", &templates.git),
            ("editor", &templates.logger),
            (OPENER, &templates.logger),
        ] {
            symlink(template, bin.join(name)).expect("link shim");
        }
        symlink("/bin/sh", bin.join("sh")).expect("link sh");
        symlink(env!("CARGO_BIN_EXE_work"), bin.join("work")).expect("link work");
        env.write_gitconfig(&["github.com", "ghe.example.com"]);
        env
    }

    pub fn root(&self) -> &Path {
        self.root.path()
    }

    pub fn home(&self) -> PathBuf {
        self.root().join("home")
    }

    pub fn work_dir(&self) -> PathBuf {
        self.home().join(WORK_DIR)
    }

    pub fn config_path(&self) -> PathBuf {
        self.home().join(".config/work/config.toml")
    }

    pub fn cache_path(&self) -> PathBuf {
        self.home().join(".local/state/work/cache.json")
    }

    pub fn frecency_path(&self) -> PathBuf {
        self.home().join(".local/state/work/frecency.json")
    }

    /// The whole environment a `work` child sees.
    pub fn vars(&self) -> Vec<(&'static str, OsString)> {
        let home = self.home();
        vec![
            ("HOME", home.clone().into()),
            ("XDG_CONFIG_HOME", home.join(".config").into()),
            ("XDG_STATE_HOME", home.join(".local/state").into()),
            ("XDG_DATA_HOME", home.join(".local/share").into()),
            ("PATH", self.root().join("bin").into()),
            ("WORK_EDITOR", self.root().join("bin/editor").into()),
            ("GIT_CONFIG_GLOBAL", self.root().join("gitconfig").into()),
            ("GIT_CONFIG_NOSYSTEM", "1".into()),
        ]
    }

    /// `work` with exactly [`TestEnv::vars`], run in `HOME`.
    pub fn command(&self) -> assert_cmd::Command {
        let mut cmd = assert_cmd::cargo_bin_cmd!("work");
        cmd.env_clear().current_dir(self.home()).envs(self.vars());
        cmd
    }

    /// Writes `config.toml` verbatim.
    pub fn config(&self, toml: &str) {
        write(&self.config_path(), toml);
    }

    /// `work_dir` plus one section per domain; the first one is the default.
    pub fn config_domains(&self, domains: &[&str]) {
        let work_dir = toml_edit::Value::from(self.work_dir().to_str().expect("utf-8"));
        let mut toml = format!("work_dir = {work_dir}\n");
        for (i, domain) in domains.iter().enumerate() {
            writeln!(toml, "\n[domains.\"{domain}\"]").expect("write to String");
            if i == 0 {
                toml.push_str("is_default = true\n");
            }
        }
        self.config(&toml);
    }

    /// The `gh api` answer for `path` on `host`, one line each.
    pub fn gh_api(&self, host: &str, path: &str, lines: &[String]) {
        let mut text = lines.join("\n");
        text.push('\n');
        write(&self.fixture(host, path, "jsonl"), &text);
    }

    /// Makes `gh api` for `path` on `host` exit 1 with `stderr`.
    pub fn gh_api_fails(&self, host: &str, path: &str, stderr: &str) {
        write(&self.fixture(host, path, "err"), stderr);
    }

    /// Fixtures for a whole domain: the viewer `me`, its member `orgs`, and
    /// every repo slug in `repos` (`org/name`, or `me/name` for a personal
    /// repo) in its owner's listing.
    pub fn gh_domain(&self, host: &str, orgs: &[&str], repos: &[&str]) {
        self.gh_api(host, "user", &["me".to_owned()]);
        let orgs: Vec<String> = orgs.iter().map(|org| (*org).to_owned()).collect();
        self.gh_api(host, "user/orgs", &orgs);
        let lines = |owner: &str| -> Vec<String> {
            repos
                .iter()
                .filter_map(|repo| repo.split_once('/'))
                .filter(|(org, _)| *org == owner)
                .map(|(org, name)| repo_line(host, org, name))
                .collect()
        };
        self.gh_api(host, "user/repos", &lines("me"));
        for org in &orgs {
            self.gh_api(host, &format!("orgs/{org}/repos"), &lines(org));
        }
    }

    /// Makes `gh auth status|login|logout --hostname <host>` exit `code`.
    pub fn gh_auth_exit(&self, host: &str, code: u8) {
        write(
            &self.root().join(format!("auth-{host}")),
            &format!("{code}\n"),
        );
    }

    /// Writes `cache.json` holding `slugs` (`host/org/repo`), with the https
    /// URLs a refresh would store; `stale` dates it in the distant past.
    pub fn seed_cache(&self, slugs: &[&str], stale: bool) {
        let repos: Vec<_> = slugs
            .iter()
            .map(|slug| {
                json!({
                    "slug": slug,
                    "clone_url": format!("https://{slug}.git"),
                    "html_url": format!("https://{slug}"),
                })
            })
            .collect();
        let updated_at = if stale {
            STALE.to_owned()
        } else {
            jiff::Timestamp::now().to_string()
        };
        let cache = json!({ "version": 2, "updated_at": updated_at, "repos": repos });
        write(&self.cache_path(), &cache.to_string());
    }

    /// Writes `frecency.json` with `(slug, access_count)` entries accessed now.
    pub fn seed_frecency(&self, entries: &[(&str, u32)]) {
        let now = jiff::Timestamp::now().to_string();
        let frecency: serde_json::Map<_, _> = entries
            .iter()
            .map(|(slug, count)| {
                (
                    (*slug).to_owned(),
                    json!({ "access_count": count, "last_access": now }),
                )
            })
            .collect();
        let file = json!({ "version": 1, "frecency": frecency, "query_frecency": {} });
        write(&self.frecency_path(), &file.to_string());
    }

    /// A bare repo that `https://<slug>.git` resolves to, with one commit
    /// on `main`.
    pub fn remote(&self, slug: &str) -> Remote<'_> {
        let (host, path) = slug.split_once('/').expect("host/org/repo");
        let remote = Remote {
            env: self,
            url: format!("https://{slug}.git"),
            bare: self.root().join(format!("remotes/{host}/{path}.git")),
            seed: self.root().join(format!("seeds/{slug}")),
        };
        fs::create_dir_all(&remote.bare).expect("create bare dir");
        fs::create_dir_all(&remote.seed).expect("create seed dir");
        self.git(&remote.bare, &["init", "--bare", "--initial-branch=main"]);
        self.git(&remote.seed, &["init", "--initial-branch=main"]);
        self.git(&remote.seed, &["remote", "add", "origin", &remote.url]);
        remote.commit("README.md", "seed\n");
        remote
    }

    /// Runs `git` with the test's global config and returns trimmed stdout.
    pub fn git(&self, dir: &Path, args: &[&str]) -> String {
        let out = Command::new(&templates().real_git)
            .current_dir(dir)
            .args(args)
            .env_clear()
            .envs(self.vars())
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "git {args:?} in {}: {}",
            dir.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    /// Lines logged by the `gh`, `git`, `editor` or opener script.
    pub fn calls(&self, name: &str) -> Vec<String> {
        let name = if name == "opener" { OPENER } else { name };
        fs::read_to_string(self.root().join(format!("{name}.log")))
            .map(|text| text.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }

    /// The pids the editor or opener script recorded.
    pub fn pids(&self, name: &str) -> Vec<u32> {
        let name = if name == "opener" { OPENER } else { name };
        fs::read_to_string(self.root().join(format!("{name}.pid")))
            .map(|text| text.lines().filter_map(|l| l.parse().ok()).collect())
            .unwrap_or_default()
    }

    fn fixture(&self, host: &str, path: &str, ext: &str) -> PathBuf {
        self.root().join(format!("fixtures/{host}/{path}.{ext}"))
    }

    fn write_gitconfig(&self, hosts: &[&str]) {
        let mut text = String::from(
            "[user]\n\tname = Test\n\temail = test@example.invalid\n\
             [init]\n\tdefaultBranch = main\n",
        );
        for host in hosts {
            let remotes = self.root().join("remotes").join(host);
            writeln!(
                text,
                "[url \"file://{}/\"]\n\tinsteadOf = https://{host}/",
                remotes.display()
            )
            .expect("write to String");
        }
        write(&self.root().join("gitconfig"), &text);
    }
}

/// A bare repo plus a private clone that advances it.
pub struct Remote<'e> {
    env: &'e TestEnv,
    url: String,
    bare: PathBuf,
    seed: PathBuf,
}

impl Remote<'_> {
    /// Commits `file` and pushes `main`; returns the new head.
    pub fn commit(&self, file: &str, content: &str) -> String {
        fs::write(self.seed.join(file), content).expect("write file");
        self.env.git(&self.seed, &["add", "--", file]);
        self.env.git(&self.seed, &["commit", "-m", file]);
        self.env.git(&self.seed, &["push", "origin", "main"]);
        self.head()
    }

    pub fn head(&self) -> String {
        self.env.git(&self.bare, &["rev-parse", "main"])
    }

    pub fn bare(&self) -> &Path {
        &self.bare
    }

    /// Clones into `dest` (creating parents) and returns `dest`.
    pub fn clone_to(&self, dest: &Path) -> PathBuf {
        fs::create_dir_all(dest.parent().expect("parent")).expect("create parents");
        let dest_str = dest.to_str().expect("utf-8");
        self.env
            .git(self.env.root(), &["clone", &self.url, dest_str]);
        dest.to_path_buf()
    }
}

/// One line of `gh api --jq` repo-listing output.
pub fn repo_line(host: &str, owner: &str, name: &str) -> String {
    json!({
        "name": name,
        "owner": owner,
        "clone_url": format!("https://{host}/{owner}/{name}.git"),
        "html_url": format!("https://{host}/{owner}/{name}"),
    })
    .to_string()
}

pub fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The path as `work` prints it: absolute, UTF-8, one line.
pub fn line(path: &Path) -> String {
    format!("{}\n", path.to_str().expect("utf-8 path"))
}

/// Polls `probe` until it returns `Some`, for a detached child that may
/// still be starting; fails the test after a few seconds.
pub fn wait_for<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(found) = probe() {
            return found;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::yield_now();
    }
}

/// Whether process `pid` is still running.
pub fn is_running(pid: u32) -> bool {
    Command::new("/bin/sh")
        .args(["-c", &format!("kill -0 {pid} 2>/dev/null")])
        .status()
        .expect("run kill")
        .success()
}

/// Every directory under `root`, sorted.
pub fn dirs_under(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                found.push(path.clone());
                stack.push(path);
            }
        }
    }
    found.sort();
    found
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("create parents");
    fs::write(path, text).expect("write file");
}

struct Templates {
    gh: PathBuf,
    git: PathBuf,
    logger: PathBuf,
    real_git: PathBuf,
}

/// The shim scripts, written once per process and only symlinked by each
/// test: exec'ing a file that another thread may still hold open for writing
/// fails with `ETXTBSY`.
fn templates() -> &'static Templates {
    static TEMPLATES: OnceLock<Templates> = OnceLock::new();
    TEMPLATES.get_or_init(|| {
        let real_git = find_on_path("git");
        let git = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"${{0%/*}}/../git.log\"\nexec '{}' \"$@\"\n",
            real_git.display()
        );
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shims");
        fs::create_dir_all(&dir).expect("create template dir");
        Templates {
            gh: install(&dir, "gh", FAKE_GH),
            git: install(&dir, "git", &git),
            logger: install(&dir, "logger", LOGGER),
            real_git,
        }
    })
}

/// Writes `script` to a content-addressed file, so concurrent test
/// processes never rewrite a script another one is running.
fn install(dir: &Path, name: &str, script: &str) -> PathBuf {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    script.hash(&mut hasher);
    let path = dir.join(format!("{name}-{:016x}", hasher.finish()));
    if !path.exists() {
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        fs::write(&tmp, script).expect("write script");
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755)).expect("chmod script");
        fs::rename(&tmp, &path).expect("install script");
    }
    path
}

fn find_on_path(program: &str) -> PathBuf {
    let path = std::env::var_os("PATH").expect("PATH is set");
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
        .and_then(|found| found.canonicalize().ok())
        .unwrap_or_else(|| panic!("`{program}` not found on PATH"))
}
