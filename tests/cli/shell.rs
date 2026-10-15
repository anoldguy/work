//! The `work activate` wrapper in real bash, zsh and fish.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::support::{TestEnv, line, repo_line, text};

/// An env whose `work_dir` holds a space, `'` and `\`, with `acme/foo`
/// cached and on its remote. Returns the work dir.
fn setup() -> (TestEnv, PathBuf) {
    let env = TestEnv::new();
    let work_dir = env.home().join(r"it's a \ dir");
    let value = toml_edit::Value::from(work_dir.to_str().expect("utf-8"));
    env.config(&format!(
        "work_dir = {value}\n[domains.\"github.com\"]\nis_default = true\n"
    ));
    env.remote("github.com/acme/foo");
    env.seed_cache(&["github.com/acme/foo"], false);
    (env, work_dir)
}

/// The shell's path on the host `PATH`. A missing shell skips the test,
/// unless `WORK_REQUIRE_SHELLS=1`.
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
        eprintln!("{shell} is not on PATH; skipping");
    }
    found
}

/// Runs `body` in `shell` without startup files, after the activation line.
fn run(env: &TestEnv, shell: &str, program: &Path, body: &str) -> Output {
    let mut cmd = Command::new(program);
    let activate = match shell {
        "fish" => {
            cmd.arg("--no-config");
            "work activate fish | source".to_owned()
        }
        "zsh" => {
            cmd.arg("-f");
            r#"eval "$(work activate zsh)""#.to_owned()
        }
        _ => {
            cmd.args(["--norc", "--noprofile"]);
            r#"eval "$(work activate bash)""#.to_owned()
        }
    };
    cmd.arg("-c")
        .arg(format!("{activate}\n{body}"))
        .env_clear()
        .envs(env.vars())
        .current_dir(env.home())
        .stdin(Stdio::null())
        .output()
        .expect("run shell")
}

fn wrapper_navigates(shell: &str) {
    let Some(program) = find_shell(shell) else {
        return;
    };
    let (env, work_dir) = setup();
    let status = if shell == "fish" { "$status" } else { "$?" };
    let home = env.home().canonicalize().expect("canonical home");

    let out = run(
        &env,
        shell,
        &program,
        &format!("work cd zzzz; echo rc={status}; pwd -P; work cd foo; pwd -P; work cd; pwd -P"),
    );
    let repo = work_dir
        .join("acme/foo")
        .canonicalize()
        .expect("cd cloned the repo");
    let work_dir = work_dir.canonicalize().expect("work dir exists");
    assert_eq!(
        text(&out.stdout),
        format!("rc=1\n{}{}{}", line(&home), line(&repo), line(&work_dir)),
        "{shell}: {}",
        text(&out.stderr)
    );

    let out = run(&env, shell, &program, "work on foo; pwd -P");
    assert_eq!(
        text(&out.stdout),
        line(&repo),
        "{shell}: {}",
        text(&out.stderr)
    );
    let repo_str = repo.to_str().expect("utf-8");
    assert_eq!(env.calls("editor"), [format!("{repo_str}\t{repo_str}")]);

    if shell != "fish" {
        let out = run(&env, shell, &program, "set -u; work >/dev/null; echo rc=$?");
        assert_eq!(
            text(&out.stdout),
            "rc=0\n",
            "{shell}: {}",
            text(&out.stderr)
        );
    }
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

#[test]
fn sh_cd_on_a_stale_cache_prints_only_the_path() {
    let env = TestEnv::new();
    env.config_domains(&["github.com"]);
    env.seed_cache(&["github.com/acme/old"], true);
    env.gh_domain("github.com", &["acme", "broken"], &["acme/foo"]);
    env.gh_api(
        "github.com",
        "orgs/acme/repos",
        &[
            repo_line("github.com", "acme", "foo"),
            "{\"name\": 1}".to_owned(),
        ],
    );
    env.gh_api_fails("github.com", "orgs/broken/repos", "HTTP 403: forbidden");
    env.remote("github.com/acme/foo");

    let out = env.command().args(["sh-cd", "foo"]).output().expect("run");
    let stderr = text(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert_eq!(text(&out.stdout), line(&env.work_dir().join("acme/foo")));
    assert!(stderr.contains("broken"), "{stderr}");
    assert!(stderr.contains("github.com/acme"), "{stderr}");
}

#[test]
fn sh_on_prints_only_the_absolute_path() {
    let env = TestEnv::new();
    env.config_domains(&["github.com"]);
    env.seed_cache(&["github.com/acme/foo"], false);
    env.remote("github.com/acme/foo");

    let out = env.command().args(["sh-on", "foo"]).output().expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), line(&env.work_dir().join("acme/foo")));
    assert_eq!(env.calls("editor"), Vec::<String>::new());
}
