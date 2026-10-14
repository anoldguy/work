//! `update` and `favorite`, run against `support::FakeApi`.

use predicates::prelude::*;

use crate::support::{DomainCfg, RepoSeed, Response, TestEnv};

fn repo_json(owner: &str, name: &str) -> String {
    format!(
        r#"{{"name":"{name}","owner":{{"login":"{owner}"}},"clone_url":"https://github.com/{owner}/{name}.git","html_url":"https://github.com/{owner}/{name}","private":false}}"#
    )
}

/// `github.com` as `me`, member of `acme` and `widgets`, one repo each.
fn healthy(env: &TestEnv) {
    let api = env.api.host("github.com");
    api.route("/user", Response::json(r#"{"login":"me"}"#));
    api.route(
        "/user/orgs",
        Response::json(r#"[{"login":"acme"},{"login":"widgets"}]"#),
    );
    api.route(
        "/user/repos",
        Response::json(&format!("[{}]", repo_json("me", "dotfiles"))),
    );
    for (org, name) in [("acme", "api"), ("widgets", "gear")] {
        api.route(
            &format!("/orgs/{org}/repos"),
            Response::json(&format!("[{}]", repo_json(org, name))),
        );
    }
}

fn config(env: &TestEnv, domain: DomainCfg) {
    env.config().domain(domain).write();
}

fn cached_slugs(env: &TestEnv) -> Vec<String> {
    let text = std::fs::read_to_string(env.cache_path()).expect("cache");
    let json: serde_json::Value = serde_json::from_str(&text).expect("json");
    let mut slugs: Vec<String> = json["repos"]
        .as_array()
        .expect("repos")
        .iter()
        .map(|r| r["slug"].as_str().expect("slug").to_owned())
        .collect();
    slugs.sort();
    slugs
}

#[test]
fn update_prints_a_summary_and_keeps_stderr_quiet() {
    let env = TestEnv::new();
    config(&env, DomainCfg::new("github.com").default_domain());
    healthy(&env);
    env.command()
        .arg("update")
        .assert()
        .success()
        .stdout("Cached 3 repositories across 1 domain\n")
        .stderr("");
    assert_eq!(
        cached_slugs(&env),
        [
            "github.com/acme/api",
            "github.com/me/dotfiles",
            "github.com/widgets/gear"
        ]
    );
}

#[test]
fn update_respects_favorites_unless_all_orgs() {
    let env = TestEnv::new();
    config(
        &env,
        DomainCfg::new("github.com")
            .default_domain()
            .favorites(&["acme"]),
    );
    healthy(&env);
    env.command()
        .arg("update")
        .assert()
        .success()
        .stdout("Cached 2 repositories across 1 domain\n");
    assert!(
        !cached_slugs(&env)
            .iter()
            .any(|s| s.starts_with("github.com/widgets"))
    );

    env.command()
        .args(["update", "--all-orgs"])
        .assert()
        .success()
        .stdout("Cached 3 repositories across 1 domain\n");
}

#[test]
fn failing_org_warns_by_name_and_keeps_its_previous_repos() {
    let env = TestEnv::new();
    config(&env, DomainCfg::new("github.com").default_domain());
    healthy(&env);
    env.api
        .host("github.com")
        .route("/orgs/widgets/repos", Response::status(500));
    env.cache()
        .repo(RepoSeed::new("github.com/widgets/old"))
        .write();
    env.command()
        .arg("update")
        .assert()
        .success()
        .stderr(predicate::str::contains("widgets").and(predicate::str::contains("warning")))
        .stdout("Cached 3 repositories across 1 domain\n");
    assert!(
        cached_slugs(&env).contains(&"github.com/widgets/old".to_owned()),
        "{:?}",
        cached_slugs(&env)
    );
}

#[test]
fn update_failing_everywhere_errors_and_leaves_the_cache_untouched() {
    let env = TestEnv::new();
    config(&env, DomainCfg::new("github.com").default_domain());
    let api = env.api.host("github.com");
    api.route("/user", Response::status(500));
    api.route("/user/orgs", Response::status(500));
    env.cache()
        .repo(RepoSeed::new("github.com/acme/old"))
        .write();
    let before = std::fs::read(env.cache_path()).expect("cache");
    env.command().arg("update").assert().failure().stdout("");
    assert_eq!(std::fs::read(env.cache_path()).expect("cache"), before);
}

#[test]
fn favorite_without_a_terminal_is_a_one_line_error() {
    let env = TestEnv::new();
    config(&env, DomainCfg::new("github.com").default_domain());
    healthy(&env);
    let assert = env.command().arg("favorite").assert().code(1).stdout("");
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
    assert!(stderr.contains("interactive terminal"), "{stderr}");
    assert_eq!(env.api.requests().len(), 0);
}
