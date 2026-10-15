//! `frecency.json`: kept apart from the cache, robust to damage.

use std::fs;

use crate::support::{DomainCfg, RepoSeed, Response, TestEnv};

/// An org listing of `acme`'s `names`, as the API returns it.
fn acme_repos(names: &[&str]) -> Response {
    let repos: Vec<String> = names
        .iter()
        .map(|name| {
            format!(
                r#"{{"name":"{name}","owner":{{"login":"acme"}},"clone_url":"https://github.com/acme/{name}.git","html_url":"https://github.com/acme/{name}","private":false}}"#
            )
        })
        .collect();
    Response::json(&format!("[{}]", repos.join(",")))
}

/// github.com as `me`, a member of `acme` only.
fn serve_acme(env: &TestEnv, names: &[&str]) {
    let api = env.api.host("github.com");
    api.route("/user", Response::json(r#"{"login":"me"}"#));
    api.route("/user/orgs", Response::json(r#"[{"login":"acme"}]"#));
    api.route("/user/repos", Response::json("[]"));
    api.route("/orgs/acme/repos", acme_repos(names));
}

fn env() -> TestEnv {
    let env = TestEnv::new();
    env.config()
        .domain(DomainCfg::new("github.com").default_domain())
        .write();
    env
}

/// `work list <query>`, one slug per line.
fn list(env: &TestEnv, query: &str) -> Vec<String> {
    let out = env.command().args(["list", query]).output().expect("run");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("utf-8")
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn frecency_survives_a_refresh_that_drops_a_repo_and_ranks_it_when_it_returns() {
    let env = env();
    let now = jiff::Timestamp::now().to_string();
    env.cache()
        .repos(&[
            RepoSeed::new("github.com/acme/app"),
            RepoSeed::new("github.com/acme/apz"),
        ])
        .frecency("github.com/acme/apz", 5, &now)
        .write();
    assert_eq!(
        list(&env, "ap"),
        ["github.com/acme/apz", "github.com/acme/app"]
    );
    let history = fs::read(env.frecency_path()).expect("frecency");

    serve_acme(&env, &["app"]);
    env.command().arg("update").assert().success();
    assert_eq!(list(&env, "ap"), ["github.com/acme/app"]);
    assert_eq!(fs::read(env.frecency_path()).expect("frecency"), history);

    serve_acme(&env, &["app", "apz"]);
    env.command().arg("update").assert().success();
    assert_eq!(
        list(&env, "ap"),
        ["github.com/acme/apz", "github.com/acme/app"]
    );
}

#[test]
fn a_corrupt_frecency_file_is_renamed_with_a_warning_and_history_restarts() {
    let env = env();
    env.seed_cache(&[RepoSeed::new("github.com/acme/app")]);
    fs::write(env.frecency_path(), "{not json").expect("write");
    let out = env.command().args(["browse", "app"]).output().expect("run");
    assert!(out.status.success());
    let stderr = String::from_utf8(out.stderr).expect("utf-8");
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(
        stderr.contains("warning") && stderr.contains("frecency.json"),
        "{stderr}"
    );
    let corrupt = env.frecency_path().with_extension("json.corrupt");
    assert_eq!(fs::read_to_string(corrupt).expect("corrupt"), "{not json");
    let history: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(env.frecency_path()).expect("new")).expect("json");
    assert_eq!(
        history["frecency"]["github.com/acme/app"]["access_count"],
        1
    );
}

#[test]
fn a_frecency_file_of_unknown_version_is_never_overwritten() {
    let env = env();
    env.seed_cache(&[RepoSeed::new("github.com/acme/app")]);
    let body = r#"{"version":7,"frecency":{"future":"shape"}}"#;
    fs::write(env.frecency_path(), body).expect("write");
    let out = env.command().args(["browse", "app"]).output().expect("run");
    assert!(out.status.success());
    let stderr = String::from_utf8(out.stderr).expect("utf-8");
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(stderr.contains("read-only"), "{stderr}");
    assert_eq!(fs::read_to_string(env.frecency_path()).expect("kept"), body);
}
