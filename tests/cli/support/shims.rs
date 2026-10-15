//! Recording executables standing in for `gh`, the editor and the opener.
//!
//! All shims are symlinks to one script template, written once per process
//! and dispatching on its own basename. Scripts are never exec'd while any
//! thread of this process may still hold a write handle to them (see
//! [`install_script`]), which would otherwise fail with `ETXTBSY` under
//! `cargo test`'s threaded runner.

use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{ErrorKind, Read};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;

/// Shell template for every shim. Uses only builtins, so it works with a
/// `PATH` that holds nothing else. State lives next to the symlink:
/// `<name>.calls/NNNNNN` (argv, NUL-terminated), `<name>.exit`,
/// `<name>.stdout`, `gh.token.<host>`, `gh.api/<host>/<path>.jsonl|.err` and
/// the `<name>.fifo` that receives one byte per recorded call.
///
/// `gh api --hostname <host> [--paginate] <path>[?query] ...` prints
/// `gh.api/<host>/<path>.jsonl`, or exits 1 with `<path>.err` on stderr, or
/// (neither present) exits 1 with a 404 message. `gh auth status --hostname
/// <host>` succeeds exactly when `gh.token.<host>` exists.
const TEMPLATE: &str = r#"#!/bin/sh
if [ -n "$WORK_SHIM_PROBE" ]; then exit 0; fi
dir=${0%/*}
name=${0##*/}
n=0
while :; do
  p=$(printf '%06d' "$n")
  if ( set -C; : > "$dir/$name.calls/$p" ) 2>/dev/null; then break; fi
  n=$((n + 1))
done
file="$dir/$name.calls/$p"
if [ "$#" -gt 0 ]; then printf '%s\0' "$@" > "$file"; fi
printf . >> "$dir/$name.fifo"
if [ "$name" = gh ] && [ "$1" = auth ] && [ "$2" = token ] && [ "$3" = --hostname ]; then
  tokfile="$dir/gh.token.$4"
  if [ -f "$tokfile" ]; then
    read -r tok < "$tokfile"
    printf '%s\n' "$tok"
    exit 0
  fi
  printf 'no oauth token found for %s\n' "$4" >&2
  exit 1
fi
if [ "$name" = gh ] && [ "$1" = api ] && [ "$2" = --hostname ]; then
  host=$3
  shift 3
  if [ "$1" = --paginate ]; then shift; fi
  fix="$dir/gh.api/$host/${1%%\?*}"
  if [ -f "$fix.err" ]; then
    while IFS= read -r line || [ -n "$line" ]; do printf '%s\n' "$line"; done < "$fix.err" >&2
    exit 1
  fi
  if [ -f "$fix.jsonl" ]; then
    while IFS= read -r line || [ -n "$line" ]; do printf '%s\n' "$line"; done < "$fix.jsonl"
    exit 0
  fi
  printf 'gh: Not Found (HTTP 404)\n' >&2
  exit 1
fi
if [ "$name" = gh ] && [ "$1" = auth ] && [ "$2" = status ] && [ "$3" = --hostname ]; then
  if [ -f "$dir/gh.token.$4" ]; then
    printf '%s\n  Logged in to %s\n' "$4" "$4"
    exit 0
  fi
  printf 'You are not logged into %s. To log in, run: gh auth login --hostname %s\n' "$4" "$4" >&2
  exit 1
fi
if [ -f "$dir/$name.stdout" ]; then
  while IFS= read -r line || [ -n "$line" ]; do printf '%s\n' "$line"; done < "$dir/$name.stdout"
fi
code=0
if [ -f "$dir/$name.exit" ]; then read -r code < "$dir/$name.exit"; fi
exit "$code"
"#;

/// Writes an executable script at `path`, safe to exec immediately.
///
/// The file is written to a private temporary name, closed, renamed into
/// place, and then run once with `WORK_SHIM_PROBE` set (scripts exit at once
/// when it is). Another thread that forked while the write handle was open
/// keeps that handle until its own `exec`, and exec'ing the script meanwhile
/// fails with `ETXTBSY`; the probe retries (yielding, never sleeping) until
/// no such handle is left. Scripts must start with the probe check.
pub(super) fn install_script(path: &Path, contents: &str) {
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&tmp, contents).expect("write script");
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755)).expect("chmod script");
    fs::rename(&tmp, path).expect("rename script into place");
    loop {
        match Command::new(path)
            .env_clear()
            .env("WORK_SHIM_PROBE", "1")
            .output()
        {
            Ok(_) => return,
            Err(err) if err.kind() == ErrorKind::ExecutableFileBusy => thread::yield_now(),
            Err(err) => panic!("probe {}: {err}", path.display()),
        }
    }
}

/// The per-process shim template, content-addressed so concurrent test
/// processes and stale builds never disagree about it.
fn template() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let mut hasher = DefaultHasher::new();
        TEMPLATE.hash(&mut hasher);
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shims");
        fs::create_dir_all(&dir).expect("create template dir");
        let path = dir.join(format!("shim-{:016x}", hasher.finish()));
        install_script(&path, TEMPLATE);
        path
    })
}

/// A shim executable in the shim directory.
///
/// Each invocation records its argv under `<name>.calls/`; stdout and the
/// exit code come from files that tests rewrite at any time.
#[derive(Debug, Clone)]
pub struct Shim {
    dir: PathBuf,
    name: String,
    /// Read end of the FIFO the shim writes one byte to per call. Opened
    /// read-write so the shim's `open` never blocks.
    fifo: Arc<File>,
    /// Call signals consumed from the FIFO so far.
    seen: Arc<AtomicUsize>,
}

impl Shim {
    /// Creates shim `name` in `dir` as a symlink to the shared template.
    pub(super) fn install(dir: &Path, name: &str) -> Self {
        fs::create_dir_all(dir.join(format!("{name}.calls"))).expect("create calls dir");
        symlink(template(), dir.join(name)).expect("link shim");
        let fifo_path = dir.join(format!("{name}.fifo"));
        let made = Command::new("mkfifo")
            .arg(&fifo_path)
            .status()
            .expect("run mkfifo");
        assert!(made.success(), "mkfifo failed");
        let fifo = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&fifo_path)
            .expect("open fifo");
        Self {
            dir: dir.to_path_buf(),
            name: name.to_owned(),
            fifo: Arc::new(fifo),
            seen: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn calls_dir(&self) -> PathBuf {
        self.dir.join(format!("{}.calls", self.name))
    }

    /// Absolute path of the executable.
    #[must_use]
    pub fn path(&self) -> PathBuf {
        self.dir.join(&self.name)
    }

    /// Makes the shim exit with `code` (default 0).
    pub fn set_exit(&self, code: i32) -> &Self {
        fs::write(
            self.dir.join(format!("{}.exit", self.name)),
            code.to_string(),
        )
        .expect("write exit code");
        self
    }

    /// Makes the shim print `text` on stdout (a trailing newline is always
    /// printed). Ignored by `gh auth token --hostname`, which uses
    /// [`Shim::set_token`].
    pub fn set_stdout(&self, text: &str) -> &Self {
        fs::write(self.dir.join(format!("{}.stdout", self.name)), text).expect("write stdout");
        self
    }

    /// For the `gh` shim: the token `gh auth token --hostname <host>` prints.
    /// Hosts without a token make the shim exit 1 with a message on stderr.
    pub fn set_token(&self, host: &str, token: &str) -> &Self {
        fs::write(self.dir.join(format!("gh.token.{host}")), token).expect("write token");
        self
    }

    /// For the `gh` shim: `gh api --hostname <host> ... <path>` prints these
    /// lines (one JSON object, or one `--jq` scalar, per line). `path` is
    /// the API path without its query, e.g. `orgs/acme/repos`.
    pub fn set_api(&self, host: &str, path: &str, lines: &[String]) -> &Self {
        let file = self
            .dir
            .join("gh.api")
            .join(host)
            .join(format!("{path}.jsonl"));
        fs::create_dir_all(file.parent().expect("parent")).expect("create api fixture dir");
        let mut text = lines.join("\n");
        text.push('\n');
        fs::write(file, text).expect("write api fixture");
        self
    }

    /// For the `gh` shim: `gh api` on `path` exits 1 with `stderr`.
    pub fn fail_api(&self, host: &str, path: &str, stderr: &str) -> &Self {
        let file = self
            .dir
            .join("gh.api")
            .join(host)
            .join(format!("{path}.err"));
        fs::create_dir_all(file.parent().expect("parent")).expect("create api fixture dir");
        fs::write(file, stderr).expect("write api fixture");
        self
    }

    /// The recorded `gh api` invocations (for the `gh` shim).
    #[must_use]
    pub fn api_calls(&self) -> Vec<Vec<String>> {
        self.calls()
            .into_iter()
            .filter(|args| args.first().is_some_and(|a| a == "api"))
            .collect()
    }

    /// Every recorded invocation, oldest first, each as its argv without the
    /// program name.
    #[must_use]
    pub fn calls(&self) -> Vec<Vec<String>> {
        let mut files: Vec<PathBuf> = fs::read_dir(self.calls_dir())
            .expect("read calls dir")
            .map(|entry| entry.expect("dir entry").path())
            .collect();
        files.sort();
        files
            .iter()
            .map(|file| {
                let raw = fs::read(file).expect("read call");
                if raw.is_empty() {
                    return Vec::new();
                }
                // Every argument ends in NUL, so the split leaves an empty tail.
                let mut args: Vec<String> = raw
                    .split(|&b| b == 0)
                    .map(|arg| String::from_utf8_lossy(arg).into_owned())
                    .collect();
                args.pop();
                args
            })
            .collect()
    }

    /// Blocks until at least `n` invocations have been recorded, then
    /// returns [`Shim::calls`]. For detached children (the opener is spawned
    /// and not waited for): the shim signals each call on a FIFO, so this
    /// needs no sleeps or polling. Hangs if the call never happens, so use
    /// it only where the command is known to launch the shim.
    #[must_use]
    pub fn wait_for_calls(&self, n: usize) -> Vec<Vec<String>> {
        let mut byte = [0_u8; 1];
        while self.seen.load(Ordering::SeqCst) < n {
            (&*self.fifo)
                .read_exact(&mut byte)
                .expect("read call signal");
            self.seen.fetch_add(1, Ordering::SeqCst);
        }
        self.calls()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(name: &str) -> (tempfile::TempDir, Shim) {
        let dir = tempfile::tempdir().expect("tempdir");
        let shim = Shim::install(dir.path(), name);
        (dir, shim)
    }

    fn run(shim: &Shim, args: &[&str]) -> std::process::Output {
        Command::new(shim.path())
            .args(args)
            .env_clear()
            .output()
            .expect("run shim")
    }

    #[test]
    fn records_argv_in_order_including_empty_and_spaced_args() {
        let (_dir, shim) = fresh("opener");
        run(&shim, &["https://x.test/a b", ""]);
        run(&shim, &[]);
        assert_eq!(
            shim.calls(),
            vec![vec!["https://x.test/a b".to_owned(), String::new()], vec![]]
        );
    }

    #[test]
    fn exit_code_and_stdout_are_configurable() {
        let (_dir, shim) = fresh("editor");
        let out = run(&shim, &[]);
        assert!(out.status.success());
        assert_eq!(out.stdout.len(), 0);
        shim.set_exit(7).set_stdout("hello\nworld");
        let out = run(&shim, &[]);
        assert_eq!(out.status.code(), Some(7));
        assert_eq!(String::from_utf8_lossy(&out.stdout), "hello\nworld\n");
    }

    #[test]
    fn gh_auth_token_prints_the_per_host_token() {
        let (_dir, gh) = fresh("gh");
        gh.set_token("github.com", "ghp_abc");
        let out = run(&gh, &["auth", "token", "--hostname", "github.com"]);
        assert_eq!(String::from_utf8_lossy(&out.stdout), "ghp_abc\n");
        assert!(out.status.success());
        let out = run(&gh, &["auth", "token", "--hostname", "other.test"]);
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(out.stdout.len(), 0);
        assert_eq!(gh.calls().len(), 2);
    }

    #[test]
    fn gh_api_prints_the_fixture_or_fails_with_its_stderr() {
        let (_dir, gh) = fresh("gh");
        gh.set_api(
            "github.com",
            "orgs/acme/repos",
            &[r#"{"name":"a\\b"}"#.to_owned()],
        );
        gh.fail_api("github.com", "user", "gh: Bad credentials (HTTP 401)\n");
        let args = [
            "api",
            "--hostname",
            "github.com",
            "--paginate",
            "orgs/acme/repos?type=all&per_page=100",
            "--jq",
            ".[]",
        ];
        let out = run(&gh, &args);
        assert!(out.status.success());
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "{\"name\":\"a\\\\b\"}\n"
        );
        let out = run(&gh, &["api", "--hostname", "github.com", "user"]);
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(
            String::from_utf8_lossy(&out.stderr),
            "gh: Bad credentials (HTTP 401)\n"
        );
        let out = run(&gh, &["api", "--hostname", "other.test", "user"]);
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(gh.api_calls().len(), 3);
    }

    #[test]
    fn wait_for_calls_returns_once_a_detached_child_has_run() {
        let (_dir, shim) = fresh("xdg-open");
        let child = Command::new(shim.path())
            .arg("https://example.test")
            .env_clear()
            .spawn()
            .expect("spawn detached");
        // Not waited on: only the FIFO tells us the child has recorded.
        let calls = shim.wait_for_calls(1);
        assert_eq!(calls, vec![vec!["https://example.test".to_owned()]]);
        drop(child);
        // Already-seen signals do not block again.
        assert_eq!(shim.wait_for_calls(1).len(), 1);
    }

    #[test]
    fn concurrent_invocations_are_all_recorded() {
        let (_dir, shim) = fresh("gh");
        thread::scope(|scope| {
            for i in 0..8 {
                let shim = &shim;
                scope.spawn(move || run(shim, &[&i.to_string()]));
            }
        });
        let mut args: Vec<String> = shim.wait_for_calls(8).into_iter().flatten().collect();
        args.sort();
        assert_eq!(args, ["0", "1", "2", "3", "4", "5", "6", "7"]);
    }
}
