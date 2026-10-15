//! `auth login|logout|status`.

use crate::support::{DomainCfg, TestEnv};

#[test]
fn login_defaults_to_github_com_and_runs_gh() {
    let env = TestEnv::new();
    env.command().args(["auth", "login"]).assert().success();
    assert_eq!(
        env.gh.calls(),
        vec![vec!["auth", "login", "--hostname", "github.com"]]
    );
}

#[test]
fn logout_passes_the_domain_through() {
    let env = TestEnv::new();
    env.command()
        .args(["auth", "logout", "--domain", "ghe.example.com"])
        .assert()
        .success();
    assert_eq!(
        env.gh.calls(),
        vec![vec!["auth", "logout", "--hostname", "ghe.example.com"]]
    );
}

#[test]
fn login_failure_is_a_one_line_error() {
    let env = TestEnv::new();
    env.gh.set_exit(4);
    let assert = env.command().args(["auth", "login"]).assert().code(1);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
    assert!(stderr.starts_with("work: gh auth login failed"), "{stderr}");
}

#[test]
fn missing_gh_is_a_one_line_error() {
    let env = TestEnv::new();
    std::fs::remove_file(env.shim_dir().join("gh")).expect("remove gh");
    let assert = env.command().args(["auth", "login"]).assert().code(1);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
    assert!(stderr.contains("'gh' is required"), "{stderr}");
}

#[test]
fn status_runs_gh_for_each_domain_and_exits_1_when_one_is_logged_out() {
    let env = TestEnv::new();
    env.config()
        .domain(DomainCfg::new("github.com").default_domain())
        .domain(DomainCfg::new("ghe.example.com"))
        .write();
    env.gh.set_token("github.com", "gho_x");
    let assert = env.command().args(["auth", "status"]).assert().code(1);
    let out = assert.get_output();
    // gh's own report reaches the terminal; work adds no error line.
    assert!(String::from_utf8_lossy(&out.stdout).contains("Logged in to github.com"));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("not logged into ghe.example.com"),
        "{stderr}"
    );
    assert!(!stderr.contains("work: "), "{stderr}");
    let mut hosts: Vec<String> = env
        .gh
        .calls()
        .into_iter()
        .map(|c| {
            assert_eq!(c[..3], ["auth", "status", "--hostname"]);
            c[3].clone()
        })
        .collect();
    hosts.sort();
    assert_eq!(hosts, ["ghe.example.com", "github.com"]);

    env.gh.set_token("ghe.example.com", "gho_y");
    env.command().args(["auth", "status"]).assert().success();
}

#[test]
fn status_without_gh_is_a_one_line_error() {
    let env = TestEnv::new();
    std::fs::remove_file(env.shim_dir().join("gh")).expect("remove gh");
    let assert = env
        .command()
        .args(["auth", "status"])
        .assert()
        .code(1)
        .stdout("");
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
    assert!(stderr.contains("'gh' is required"), "{stderr}");
}

#[test]
fn login_and_logout_work_with_a_broken_config() {
    let env = TestEnv::new();
    env.write_config_raw("[domains.github.com]\nis_default = true\n");
    env.command().args(["auth", "login"]).assert().success();
    env.command().args(["auth", "logout"]).assert().success();
    assert_eq!(
        env.gh.calls(),
        vec![
            vec!["auth", "login", "--hostname", "github.com"],
            vec!["auth", "logout", "--hostname", "github.com"],
        ]
    );
}
