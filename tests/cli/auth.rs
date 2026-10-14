//! `auth login|logout|status`.

use predicates::prelude::*;

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
fn status_reports_the_env_token_source() {
    let env = TestEnv::new();
    env.command()
        .args(["auth", "status"])
        .assert()
        .success()
        .stdout("github.com: token found (GITHUB_TOKEN)\n")
        .stderr("");
    assert_eq!(env.gh.calls().len(), 0);
}

#[test]
fn status_reports_an_invalid_env_token_and_exits_1_without_an_error_line() {
    let env = TestEnv::new().token(Some("bad token"));
    env.command()
        .args(["auth", "status"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("GITHUB_TOKEN is set but invalid"))
        .stderr("");
}

#[test]
fn status_lists_each_domain_and_exits_1_when_one_lacks_a_token() {
    let env = TestEnv::new().token(None);
    env.config()
        .domain(DomainCfg::new("github.com").default_domain())
        .domain(DomainCfg::new("ghe.example.com"))
        .write();
    env.gh.set_token("github.com", "gho_x");
    env.command()
        .args(["auth", "status"])
        .assert()
        .code(1)
        .stdout(
            "ghe.example.com: no token (run 'gh auth login --hostname ghe.example.com')\n\
             github.com: token found (gh)\n",
        )
        .stderr("");
}

#[test]
fn status_with_all_tokens_from_gh_exits_0() {
    let env = TestEnv::new().token(None);
    env.gh.set_token("github.com", "gho_x");
    env.command()
        .args(["auth", "status"])
        .assert()
        .success()
        .stdout("github.com: token found (gh)\n");
}

#[test]
fn status_without_gh_or_env_token_is_a_one_line_error() {
    let env = TestEnv::new().token(None);
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
