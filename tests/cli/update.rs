//! Refreshing the cache: failures are contained and the cache is never lost.

use std::fs;

use serde_json::Value;

use crate::support::{TestEnv, text};

fn cached_slugs(env: &TestEnv) -> Vec<String> {
    let cache: Value =
        serde_json::from_str(&fs::read_to_string(env.cache_path()).expect("cache")).expect("json");
    let mut slugs: Vec<String> = cache["repos"]
        .as_array()
        .expect("repos")
        .iter()
        .map(|repo| repo["slug"].as_str().expect("slug").to_owned())
        .collect();
    slugs.sort();
    slugs
}

#[test]
fn a_failing_org_keeps_its_repos_with_one_warning() {
    let env = TestEnv::new();
    env.config_domains(&["github.com"]);
    env.seed_cache(&["github.com/acme/gone", "github.com/broken/kept"], false);
    env.gh_domain("github.com", &["acme", "broken"], &["acme/app", "me/dots"]);
    env.gh_api_fails(
        "github.com",
        "orgs/broken/repos",
        "gh: Resource protected by organization SAML enforcement (HTTP 403)",
    );

    let out = env.command().arg("update").output().expect("run");
    let stderr = text(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert_eq!(
        stderr.lines().filter(|l| l.contains("broken")).count(),
        1,
        "{stderr}"
    );
    assert_eq!(
        cached_slugs(&env),
        [
            "github.com/acme/app",
            "github.com/broken/kept",
            "github.com/me/dots"
        ]
    );
}

#[test]
fn every_domain_failing_is_an_error_and_leaves_the_cache_untouched() {
    let env = TestEnv::new();
    env.config_domains(&["github.com", "ghe.example.com"]);
    env.seed_cache(&["github.com/acme/app"], true);
    let before = fs::read(env.cache_path()).expect("cache");
    for host in ["github.com", "ghe.example.com"] {
        env.gh_api_fails(
            host,
            "user",
            "gh: To get started with GitHub CLI, please run: gh auth login",
        );
    }

    let out = env.command().arg("update").output().expect("run");
    assert!(!out.status.success());
    assert_eq!(fs::read(env.cache_path()).expect("cache"), before);
}

#[test]
fn favorites_narrow_the_listing_unless_all_orgs_and_excludes_always_apply() {
    let env = TestEnv::new();
    let work_dir = toml_edit::Value::from(env.work_dir().to_str().expect("utf-8"));
    env.config(&format!(
        "work_dir = {work_dir}\n\
         [filters]\nexclude_orgs = [\"junk\"]\n\
         [domains.\"github.com\"]\nis_default = true\nfavorite_orgs = [\"acme\", \"junk\"]\n"
    ));
    env.gh_domain(
        "github.com",
        &["acme", "beta", "junk"],
        &["acme/app", "beta/lib", "junk/old", "me/dots"],
    );

    let out = env.command().arg("update").output().expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        cached_slugs(&env),
        ["github.com/acme/app", "github.com/me/dots"]
    );

    let out = env
        .command()
        .args(["update", "--all-orgs"])
        .output()
        .expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        cached_slugs(&env),
        [
            "github.com/acme/app",
            "github.com/beta/lib",
            "github.com/me/dots"
        ]
    );
}

#[test]
fn a_failed_refresh_of_a_stale_cache_warns_and_uses_it() {
    let env = TestEnv::new();
    env.config_domains(&["github.com"]);
    env.seed_cache(&["github.com/acme/app"], true);
    let before = fs::read(env.cache_path()).expect("cache");
    env.gh_api_fails("github.com", "user", "gh: HTTP 401: Bad credentials");
    env.remote("github.com/acme/app");

    let out = env.command().args(["clone", "app"]).output().expect("run");
    let stderr = text(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(env.work_dir().join("acme/app/.git").is_dir());
    assert!(stderr.contains("cache"), "{stderr}");
    assert_eq!(fs::read(env.cache_path()).expect("cache"), before);
}
