//! The wrapper `work activate` prints, loaded the way users load it, driving
//! the built binary in bash, zsh and fish. A missing shell skips its test,
//! unless `WORK_REQUIRE_SHELLS=1` (CI), which makes it fail.

use std::ffi::OsString;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::support::{RepoSeed, TestEnv};

const SLUG: &str = "github.com/acme/foo";

/// A test env whose `work_dir` holds a space, `'`, `\` and other shell
/// metacharacters, with a fresh cache holding `acme/foo` (cloned from a
/// local origin), and a `bin/` holding only the built `work`. Returns the
/// env, the bin dir and the repo's path.
fn setup() -> (TestEnv, PathBuf, PathBuf) {
    let env = TestEnv::new();
    let work_dir = env.home().join(r#"it's a \ "$dir" `x` $(y)"#);
    let toml_value = work_dir
        .to_str()
        .expect("utf-8")
        .replace('\\', r"\\")
        .replace('"', "\\\"");
    env.write_config_raw(&format!("work_dir = \"{toml_value}\"\n"));
    let origin = env.origin("acme", "foo");
    env.seed_cache(&[RepoSeed::new(SLUG).origin(&origin)]);
    let bin = env.root().join("bin");
    std::fs::create_dir(&bin).expect("bin dir");
    symlink(env!("CARGO_BIN_EXE_work"), bin.join("work")).expect("link work");
    (env, bin, work_dir.join("acme/foo"))
}

/// `shell`'s absolute path on the host `PATH` (the shell itself then runs
/// with a `PATH` that does not hold it). `None` skips the test, unless
/// `WORK_REQUIRE_SHELLS=1`, which fails it instead.
fn find_shell(shell: &str) -> Option<PathBuf> {
    let found = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(shell))
            .find(|candidate| candidate.is_file())
    });
    if found.is_none() {
        assert!(
            std::env::var_os("WORK_REQUIRE_SHELLS").is_none_or(|v| v != "1"),
            "{shell} is not on PATH and WORK_REQUIRE_SHELLS=1"
        );
        eprintln!("note: {shell} not on PATH; skipping wrapper test");
    }
    found
}

/// Runs `body` in `shell` (at `program`, without user startup files) after
/// loading the wrapper with the documented activation line, with the test
/// env's variables and `PATH` = the work bin dir, then the shims.
fn in_shell(env: &TestEnv, bin: &Path, shell: &str, program: &Path, body: &str) -> Output {
    let mut cmd = Command::new(program);
    match shell {
        "fish" => cmd.arg("--no-config"),
        "zsh" => cmd.arg("-f"),
        _ => cmd.args(["--norc", "--noprofile"]),
    };
    let activate = match shell {
        "fish" => "work activate fish | source".to_owned(),
        _ => format!(r#"eval "$(work activate {shell})""#),
    };
    cmd.arg("-c")
        .arg(format!("{activate}\n{body}"))
        .env_clear()
        .current_dir(env.home());
    for (name, value) in env.vars() {
        cmd.env(name, value);
    }
    let mut path = OsString::from(bin);
    path.push(":");
    path.push(env.shim_dir());
    cmd.env("PATH", path)
        .stdin(Stdio::null())
        .output()
        .expect("run shell")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// In `shell`, through the wrapper, with a repo path full of quoting
/// hazards: `work cd <q>` changes directory; a failed resolution returns
/// non-zero and stays put; `work cd` goes to the work dir; `work on <q>`
/// changes directory and runs the editor there; other commands pass
/// through.
fn wrapper_navigates(shell: &str) {
    let Some(program) = find_shell(shell) else {
        return;
    };
    let run = |env: &TestEnv, bin: &Path, body: &str| in_shell(env, bin, shell, &program, body);
    let (env, bin, repo) = setup();
    let status = if shell == "fish" { "$status" } else { "$?" };

    let out = run(
        &env,
        &bin,
        &format!("work cd foo; pwd -P; work cd zzzz; echo rc={status}; pwd -P"),
    );
    let repo = repo.canonicalize().expect("canonical repo");
    let repo = repo.to_str().expect("utf-8");
    assert_eq!(
        text(&out.stdout),
        format!("{repo}\nrc=1\n{repo}\n"),
        "{shell}: stderr {}",
        text(&out.stderr)
    );

    let out = run(&env, &bin, "cd /; work cd; pwd -P; work version");
    let work_dir = Path::new(repo)
        .parent()
        .and_then(Path::parent)
        .expect("work dir");
    assert_eq!(
        text(&out.stdout),
        format!("{}\n{}\n", work_dir.display(), env!("CARGO_PKG_VERSION")),
        "{shell}: stderr {}",
        text(&out.stderr)
    );

    let out = run(&env, &bin, "work on foo; pwd -P");
    assert_eq!(
        text(&out.stdout),
        format!("{repo}\n"),
        "{shell}: stderr {}",
        text(&out.stderr)
    );
    assert_eq!(env.editor.calls(), [vec![repo.to_owned()]]);
}

#[test]
fn bash_wrapper_navigates() {
    wrapper_navigates("bash");
}

#[test]
fn zsh_wrapper_navigates() {
    wrapper_navigates("zsh");
}

#[test]
fn fish_wrapper_navigates() {
    wrapper_navigates("fish");
}

/// `work on` with an unknown repo leaves the directory alone, returns
/// non-zero, and never starts the editor.
fn failed_on_runs_nothing(shell: &str) {
    let Some(program) = find_shell(shell) else {
        return;
    };
    let (env, bin, _repo) = setup();
    let status = if shell == "fish" { "$status" } else { "$?" };
    let out = in_shell(
        &env,
        &bin,
        shell,
        &program,
        &format!("work on zzzz; echo rc={status}; pwd -P"),
    );
    let home = env.home().canonicalize().expect("canonical home");
    assert_eq!(
        text(&out.stdout),
        format!("rc=1\n{}\n", home.display()),
        "{shell}: stderr {}",
        text(&out.stderr)
    );
    assert_eq!(env.editor.calls(), Vec::<Vec<String>>::new());
}

#[test]
fn bash_failed_on_runs_nothing() {
    failed_on_runs_nothing("bash");
}

#[test]
fn zsh_failed_on_runs_nothing() {
    failed_on_runs_nothing("zsh");
}

#[test]
fn fish_failed_on_runs_nothing() {
    failed_on_runs_nothing("fish");
}
