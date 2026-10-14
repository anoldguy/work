//! The real wrappers in `shell/` evaluating the built binary's `sh-*`
//! output (W5.1): `work cd <q>` must change the calling shell's directory,
//! including for paths full of quoting hazards.

use std::ffi::OsString;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::support::{RepoSeed, TestEnv};

const SLUG: &str = "github.com/acme/foo";

/// A test env with a quote-heavy `work_dir`, a fresh cache holding
/// `acme/foo` (cloned from a local origin), and a `bin/` holding only the
/// built `work`. Returns the env, the bin dir and the repo's path.
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
/// with a `PATH` that does not hold it), or `None` when not installed.
fn find_shell(shell: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(shell))
        .find(|candidate| candidate.is_file())
}

/// Runs `body` in `shell` (at `program`) after sourcing the repo's
/// wrapper, with the test env's variables and `PATH` = the work bin dir,
/// then the shims.
fn in_shell(env: &TestEnv, bin: &Path, shell: &str, program: &Path, body: &str) -> Output {
    let wrapper = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("shell")
        .join(format!("work.{shell}"));
    let wrapper = quote(wrapper.to_str().expect("utf-8"));
    let mut cmd = Command::new(program);
    let script = match shell {
        "fish" => {
            cmd.arg("--no-config");
            format!("source {wrapper}; {body}")
        }
        "zsh" => {
            cmd.arg("-f");
            format!(". {wrapper}; {body}")
        }
        _ => {
            cmd.args(["--norc", "--noprofile"]);
            format!(". {wrapper}; {body}")
        }
    };
    cmd.arg("-c")
        .arg(script)
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

/// POSIX-and-fish single quoting for the test's own command text.
fn quote(s: &str) -> String {
    let inner: String = s
        .chars()
        .map(|c| match c {
            '\'' => r"'\''".to_owned(),
            '\\' => r"'\\'".to_owned(),
            c => c.to_string(),
        })
        .collect();
    format!("'{inner}'")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// `work cd foo; pwd` lands in the clone; a failed `work cd` leaves the
/// directory alone and reports failure; `work on foo` changes directory and
/// runs the editor on the path.
fn wrapper_navigates(shell: &str) {
    let Some(program) = find_shell(shell) else {
        eprintln!("note: {shell} not on PATH; skipping wrapper eval test");
        return;
    };
    let run = |env: &TestEnv, bin: &Path, body: &str| in_shell(env, bin, shell, &program, body);
    let (env, bin, repo) = setup();
    let body = if shell == "fish" {
        "work cd foo; pwd -P; work cd zzzz; or echo failed; pwd -P"
    } else {
        "work cd foo; pwd -P; work cd zzzz || echo failed; pwd -P"
    };
    let out = run(&env, &bin, body);
    let repo = repo.canonicalize().expect("canonical repo");
    let repo = repo.to_str().expect("utf-8");
    assert_eq!(
        text(&out.stdout),
        format!("{repo}\nfailed\n{repo}\n"),
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

    let home = env.home().canonicalize().expect("canonical home");
    let out = run(&env, &bin, "cd /; work cd; pwd -P");
    let work_dir = Path::new(repo)
        .parent()
        .and_then(Path::parent)
        .expect("work dir");
    assert!(work_dir.starts_with(&home));
    assert_eq!(
        text(&out.stdout),
        format!("{}\n", work_dir.display()),
        "{shell}: stderr {}",
        text(&out.stderr)
    );
}

#[test]
fn bash_wrapper_changes_directory() {
    wrapper_navigates("bash");
}

#[test]
fn zsh_wrapper_changes_directory() {
    wrapper_navigates("zsh");
}

#[test]
fn fish_wrapper_changes_directory() {
    wrapper_navigates("fish");
}

/// An editor value with an embedded command line: the bash wrapper must
/// run nothing (no editor, no injected command) and stay put.
#[test]
fn bash_wrapper_runs_nothing_for_an_editor_with_a_newline() {
    let Some(program) = find_shell("bash") else {
        eprintln!("note: bash not on PATH; skipping wrapper eval test");
        return;
    };
    let (env, bin, repo) = setup();
    let body = r#"export WORK_EDITOR="$(printf '%s\necho INJECTED' "$WORK_EDITOR")"; work on foo; echo "rc=$?"; pwd -P"#;
    let out = in_shell(&env, &bin, "bash", &program, body);
    let home = env.home().canonicalize().expect("canonical home");
    assert_eq!(
        text(&out.stdout),
        format!("rc=1\n{}\n", home.display()),
        "stderr {}",
        text(&out.stderr)
    );
    assert_eq!(
        text(&out.stderr).lines().count(),
        1,
        "{}",
        text(&out.stderr)
    );
    assert_eq!(env.editor.calls(), Vec::<Vec<String>>::new());
    assert!(!repo.exists());
}

#[test]
fn commands_sh_names_have_matching_sh_subcommands() {
    let env = TestEnv::new();
    let out = env
        .command()
        .args(["commands", "--sh"])
        .output()
        .expect("run");
    assert!(out.status.success());
    let names = text(&out.stdout);
    assert_eq!(names, "cd\non\n");
    for name in names.lines() {
        let help = env
            .command()
            .args([&format!("sh-{name}"), "--help"])
            .output()
            .expect("run");
        assert!(help.status.success(), "sh-{name}");
    }
}
