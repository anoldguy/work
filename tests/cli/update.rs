//! `update` and `favorite`, run against the `gh` shim's API fixtures.

use predicates::prelude::*;

use crate::support::{DomainCfg, RepoSeed, TestEnv, repo_line};

/// `github.com` as `me`, member of `acme` and `widgets`, one repo each.
fn healthy(env: &TestEnv) {
    let gh = &env.gh;
    gh.set_api("github.com", "user", &["me".to_owned()]);
    gh.set_api(
        "github.com",
        "user/orgs",
        &["acme".to_owned(), "widgets".to_owned()],
    );
    gh.set_api(
        "github.com",
        "user/repos",
        &[repo_line("github.com", "me", "dotfiles")],
    );
    for (org, name) in [("acme", "api"), ("widgets", "gear")] {
        gh.set_api(
            "github.com",
            &format!("orgs/{org}/repos"),
            &[repo_line("github.com", org, name)],
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
fn update_lists_every_source_through_gh_api() {
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
    let calls = env.gh.api_calls();
    assert!(
        calls
            .iter()
            .all(|c| c[..3] == ["api", "--hostname", "github.com"]),
        "{calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|c| c.contains(&"orgs/acme/repos?type=all&per_page=100".to_owned())),
        "{calls:?}"
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
    env.command()
        .args(["update", "--all-orgs"])
        .assert()
        .success()
        .stdout("Cached 3 repositories across 1 domain\n");
}

#[test]
fn failing_org_warns_once_and_keeps_its_previous_repos() {
    let env = TestEnv::new();
    config(&env, DomainCfg::new("github.com").default_domain());
    healthy(&env);
    env.gh.fail_api(
        "github.com",
        "orgs/widgets/repos",
        "gh: Server Error (HTTP 500)\n",
    );
    env.cache()
        .repo(RepoSeed::new("github.com/widgets/old"))
        .write();
    let assert = env
        .command()
        .arg("update")
        .assert()
        .success()
        .stdout("Cached 3 repositories across 1 domain\n");
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
    assert!(
        stderr.starts_with("warning: ")
            && stderr.contains("github.com/widgets")
            && stderr.contains("HTTP 500"),
        "{stderr}"
    );
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
    env.gh
        .fail_api("github.com", "user", "gh: Bad credentials (HTTP 401)\n");
    env.cache()
        .repo(RepoSeed::new("github.com/acme/old"))
        .write();
    let before = std::fs::read(env.cache_path()).expect("cache");
    env.command()
        .arg("update")
        .assert()
        .failure()
        .stdout("")
        .stderr(predicate::str::contains("HTTP 401"));
    assert_eq!(std::fs::read(env.cache_path()).expect("cache"), before);
}

#[test]
fn cd_on_a_stale_cache_refreshes_before_resolving() {
    let env = TestEnv::new();
    config(&env, DomainCfg::new("github.com").default_domain());
    healthy(&env);
    env.seed_stale_cache(&[RepoSeed::new("github.com/acme/old")]);
    // Already cloned, so resolving needs no clone.
    let dest = env.repo_path("github.com/widgets/gear");
    std::fs::create_dir_all(dest.join(".git")).expect("fake clone");
    let out = env
        .command()
        .args(["sh-cd", "gear"])
        .output()
        .expect("run work");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.lines().count(), 1, "{stdout:?}");
    assert!(stdout.contains("widgets/gear"), "{stdout}");
    assert_ne!(env.gh.api_calls().len(), 0);
    assert!(
        cached_slugs(&env).contains(&"github.com/widgets/gear".to_owned()),
        "{:?}",
        cached_slugs(&env)
    );
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
    assert_eq!(env.gh.api_calls().len(), 0);
}
