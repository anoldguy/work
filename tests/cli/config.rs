//! A broken config blocks only the commands that need it.

use crate::support::{TestEnv, line, text};

#[test]
fn invalid_config_fails_cd_in_one_line_but_not_config_free_commands() {
    let env = TestEnv::new();
    env.config("work_dir = [\"not\", \"a\", \"path\"\n");

    let out = env.command().args(["sh-cd", "foo"]).output().expect("run");
    assert!(!out.status.success());
    assert_eq!(text(&out.stdout), "");
    assert_eq!(
        text(&out.stderr).lines().count(),
        1,
        "{}",
        text(&out.stderr)
    );

    let out = env.command().arg("version").output().expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains(env!("CARGO_PKG_VERSION")));

    let out = env
        .command()
        .args(["activate", "bash"])
        .output()
        .expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("sh-cd"));

    let out = env
        .command()
        .args(["completions", "bash"])
        .output()
        .expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("browse"));

    let out = env
        .command()
        .args(["config", "path"])
        .output()
        .expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), line(&env.config_path()));

    let out = env.command().arg("sh-editor").output().expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    let home = env.home().canonicalize().expect("canonical home");
    let home = home.to_str().expect("utf-8");
    assert_eq!(env.calls("editor"), [format!("{home}\t{home}")]);
}
