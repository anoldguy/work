//! Reaching readable repos that no listing returns: lookup, adoption,
//! re-checking and `forget`.

use std::thread;

use crate::support::{TestEnv, line, repo_line, text, wait_for};

/// A github.com home whose fresh cache lists only `cached`.
fn home_with(cached: &[&str]) -> TestEnv {
    let env = TestEnv::new();
    env.config_domains(&["github.com"]);
    env.seed_cache(cached, false);
    env
}

/// Makes `repos/<org>/<name>` answer with `answer` (`org/name`) on `host`.
fn readable(env: &TestEnv, host: &str, asked: &str, answer: &str) {
    let (org, name) = answer.split_once('/').expect("org/name");
    env.gh_api(
        host,
        &format!("repos/{asked}"),
        &[repo_line(host, org, name)],
    );
}

fn lookups(env: &TestEnv) -> usize {
    env.calls("gh")
        .iter()
        .filter(|call| call.contains(" repos/"))
        .count()
}

fn listed(env: &TestEnv) -> Vec<String> {
    let out = env.command().arg("list").output().expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    text(&out.stdout).lines().map(str::to_owned).collect()
}

/// The URL the detached opener was given.
fn opened(env: &TestEnv) -> String {
    wait_for("the opener", || env.calls("opener").pop())
}

fn browse(env: &TestEnv, query: &str) -> std::process::Output {
    env.command().args(["browse", query]).output().expect("run")
}

#[test]
fn cd_reaches_a_readable_repo_the_cache_does_not_list_and_remembers_it() {
    let env = home_with(&["github.com/acme/app"]);
    readable(&env, "github.com", "other/lib", "other/lib");
    env.remote("github.com/other/lib");

    let out = env
        .command()
        .args(["sh-cd", "other/lib"])
        .output()
        .expect("run");
    let stderr = text(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    let dest = env.work_dir().join("other/lib");
    assert_eq!(text(&out.stdout), line(&dest));
    assert!(dest.join(".git").is_dir());
    assert!(stderr.contains("adopted github.com/other/lib"), "{stderr}");

    let again = env
        .command()
        .args(["sh-cd", "other/lib"])
        .output()
        .expect("run");
    assert!(again.status.success(), "{}", text(&again.stderr));
    assert_eq!(text(&again.stdout), line(&dest));
    assert!(!text(&again.stderr).contains("adopted"));
    assert_eq!(lookups(&env), 1);
    assert!(listed(&env).contains(&"github.com/other/lib".to_owned()));
}

#[test]
fn a_substring_match_in_the_cache_does_not_stop_the_lookup() {
    let env = home_with(&["github.com/acme/foobar"]);
    readable(&env, "github.com", "acme/foo", "acme/foo");

    let out = browse(&env, "acme/foo");
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(opened(&env).ends_with("https://github.com/acme/foo"));
}

#[test]
fn a_failed_lookup_falls_back_to_the_cached_near_miss() {
    let env = home_with(&["github.com/acme/foobar"]);

    let out = browse(&env, "acme/foo");
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(opened(&env).ends_with("https://github.com/acme/foobar"));
}

#[test]
fn a_repo_github_does_not_know_is_not_found_with_an_access_hint() {
    let env = home_with(&["github.com/acme/app"]);

    let out = browse(&env, "acme/nope");
    let stderr = text(&out.stderr);
    assert!(!out.status.success());
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(
        stderr.contains("repository not found: acme/nope"),
        "{stderr}"
    );
    assert!(
        stderr.contains("gh auth status --hostname github.com"),
        "{stderr}"
    );
}

#[test]
fn a_renamed_repo_is_adopted_under_its_current_name() {
    let env = home_with(&["github.com/acme/app"]);
    readable(&env, "github.com", "acme/old", "acme/new");

    let out = browse(&env, "acme/old");
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(opened(&env).ends_with("https://github.com/acme/new"));
    let repos = listed(&env);
    assert!(
        repos.contains(&"github.com/acme/new".to_owned()),
        "{repos:?}"
    );
    assert!(
        !repos.contains(&"github.com/acme/old".to_owned()),
        "{repos:?}"
    );
}

#[test]
fn sso_enforcement_is_a_one_line_actionable_error() {
    let env = home_with(&["github.com/acme/app"]);
    env.gh_api_fails(
        "github.com",
        "repos/corp/secret",
        "gh: Resource protected by organization SAML enforcement. You must grant your OAuth token access to this organization. (HTTP 403)",
    );

    let out = browse(&env, "corp/secret");
    let stderr = text(&out.stderr);
    assert!(!out.status.success());
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(stderr.contains("SAML"), "{stderr}");
    assert!(
        stderr.contains("gh auth status --hostname github.com"),
        "{stderr}"
    );
    assert_eq!(env.calls("opener"), Vec::<String>::new());
}

#[test]
fn hits_on_several_domains_are_ambiguous() {
    let env = TestEnv::new();
    env.config_domains(&["github.com", "ghe.example.com"]);
    env.seed_cache(&["github.com/acme/app"], false);
    readable(&env, "github.com", "acme/foo", "acme/foo");
    readable(&env, "ghe.example.com", "acme/foo", "acme/foo");

    let out = browse(&env, "acme/foo");
    let stderr = text(&out.stderr);
    assert!(!out.status.success());
    assert!(stderr.contains("ambiguous"), "{stderr}");
    assert!(stderr.contains("github.com/acme/foo"), "{stderr}");
    assert!(stderr.contains("ghe.example.com/acme/foo"), "{stderr}");
    assert_eq!(env.calls("opener"), Vec::<String>::new());
}

#[test]
fn update_keeps_adopted_repos_and_drops_those_that_are_gone() {
    let env = TestEnv::new();
    env.config_domains(&["github.com"]);
    env.gh_domain("github.com", &["acme"], &["acme/app"]);
    for name in ["kept", "gone", "flaky"] {
        let repo = format!("other/{name}");
        readable(&env, "github.com", &repo, &repo);
        let out = browse(&env, &repo);
        assert!(out.status.success(), "{}", text(&out.stderr));
    }
    env.gh_api_fails("github.com", "repos/other/gone", "gh: HTTP 404");
    env.gh_api_fails(
        "github.com",
        "repos/other/flaky",
        "gh: HTTP 502: Bad Gateway",
    );

    let out = env.command().arg("update").output().expect("run");
    let stderr = text(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(stderr.contains("github.com/other/gone"), "{stderr}");
    assert_eq!(
        listed(&env),
        [
            "github.com/acme/app",
            "github.com/other/flaky",
            "github.com/other/kept"
        ]
    );
}

#[test]
fn concurrent_adoptions_lose_nothing() {
    let env = home_with(&["github.com/acme/app"]);
    let repos: Vec<String> = (0..6).map(|i| format!("other/r{i}")).collect();
    for repo in &repos {
        readable(&env, "github.com", repo, repo);
    }

    thread::scope(|s| {
        for repo in &repos {
            let env = &env;
            s.spawn(move || {
                let out = browse(env, repo);
                assert!(out.status.success(), "{}", text(&out.stderr));
            });
        }
    });

    let all = listed(&env);
    for repo in &repos {
        assert!(all.contains(&format!("github.com/{repo}")), "{all:?}");
    }
}

#[test]
fn bare_and_malformed_queries_never_call_gh() {
    let env = home_with(&["github.com/acme/app"]);

    for query in ["lib", "ac me/lib", "acme/l*b", "nope.example.com/acme/lib"] {
        let out = browse(&env, query);
        assert!(!out.status.success(), "{query}");
    }
    assert_eq!(env.calls("gh"), Vec::<String>::new());
}

#[test]
fn forget_removes_an_adopted_repo() {
    let env = home_with(&["github.com/acme/app"]);
    readable(&env, "github.com", "other/lib", "other/lib");
    assert!(browse(&env, "other/lib").status.success());

    let out = env
        .command()
        .args(["forget", "other/lib"])
        .output()
        .expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "Forgot github.com/other/lib\n");
    assert_eq!(listed(&env), ["github.com/acme/app"]);

    let cached = env
        .command()
        .args(["forget", "acme/app"])
        .output()
        .expect("run");
    assert!(!cached.status.success());
    assert!(text(&cached.stderr).contains("no adopted repository matches"));
}

#[test]
fn a_rate_limited_lookup_is_not_reported_as_refused_access() {
    let env = home_with(&["github.com/acme/app"]);
    env.gh_api_fails(
        "github.com",
        "repos/corp/lib",
        "gh: API rate limit exceeded for user ID 1. (HTTP 403)",
    );

    let out = browse(&env, "corp/lib");
    let stderr = text(&out.stderr);
    assert!(!out.status.success());
    assert!(
        stderr.contains("cannot look up github.com/corp/lib"),
        "{stderr}"
    );
    assert!(!stderr.contains("SAML"), "{stderr}");
}

#[test]
fn a_404_without_a_message_is_not_found() {
    let env = home_with(&["github.com/acme/app"]);
    env.gh_api_fails("github.com", "repos/corp/lib", "gh: HTTP 404");

    let out = browse(&env, "corp/lib");
    let stderr = text(&out.stderr);
    assert!(!out.status.success());
    assert!(
        stderr.contains("repository not found: corp/lib"),
        "{stderr}"
    );
    assert!(
        stderr.contains("gh auth status --hostname github.com"),
        "{stderr}"
    );
}

#[test]
fn refused_access_fails_instead_of_landing_on_a_cached_near_miss() {
    let env = home_with(&["github.com/corp/secret-tools"]);
    env.gh_api_fails(
        "github.com",
        "repos/corp/secret",
        "gh: Resource protected by organization SAML enforcement. (HTTP 403)",
    );

    let out = env
        .command()
        .args(["sh-cd", "corp/secret"])
        .output()
        .expect("run");
    let stderr = text(&out.stderr);
    assert!(!out.status.success());
    assert_eq!(text(&out.stdout), "");
    assert!(
        stderr.contains("cannot read github.com/corp/secret"),
        "{stderr}"
    );
    assert!(!env.frecency_path().exists());
}

#[test]
fn update_moves_a_renamed_adopted_repo_to_its_current_name() {
    let env = TestEnv::new();
    env.config_domains(&["github.com"]);
    env.gh_domain("github.com", &["acme"], &["acme/app"]);
    readable(&env, "github.com", "other/old", "other/old");
    assert!(browse(&env, "other/old").status.success());
    readable(&env, "github.com", "other/old", "other/new");

    let out = env.command().arg("update").output().expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        listed(&env),
        ["github.com/acme/app", "github.com/other/new"]
    );
}

#[test]
fn pull_and_sync_never_look_anything_up() {
    let env = home_with(&["github.com/acme/app"]);
    readable(&env, "github.com", "other/lib", "other/lib");
    assert!(browse(&env, "other/lib").status.success());

    let pull = env
        .command()
        .args(["pull", "corp/nope"])
        .output()
        .expect("run");
    assert!(!pull.status.success());
    let sync = env.command().arg("sync").output().expect("run");
    assert!(sync.status.success(), "{}", text(&sync.stderr));
    assert_eq!(lookups(&env), 1);
}

#[test]
fn an_adopted_repo_ranks_by_frecency_like_a_cached_one() {
    let env = home_with(&["github.com/acme/lib-a", "github.com/acme/lib-b"]);
    readable(&env, "github.com", "other/lib", "other/lib");
    for _ in 0..3 {
        assert!(browse(&env, "other/lib").status.success());
    }

    let out = env.command().args(["list", "lib"]).output().expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout).lines().next(),
        Some("github.com/other/lib")
    );
}

#[test]
fn a_full_slug_query_looks_up_only_its_domain() {
    let env = TestEnv::new();
    env.config_domains(&["github.com", "ghe.example.com"]);
    env.seed_cache(&["github.com/acme/app"], false);
    readable(&env, "ghe.example.com", "corp/int", "corp/int");

    let out = browse(&env, "ghe.example.com/corp/int");
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(opened(&env).ends_with("https://ghe.example.com/corp/int"));
    let calls = env.calls("gh");
    assert!(
        calls
            .iter()
            .all(|call| !call.contains("--hostname github.com")),
        "{calls:?}"
    );
}

#[test]
fn completions_offer_adopted_repos_and_forget_offers_only_those() {
    let env = home_with(&["github.com/acme/app"]);
    readable(&env, "github.com", "other/lib", "other/lib");
    assert!(browse(&env, "other/lib").status.success());

    let complete = |extra: &[&str]| {
        let out = env
            .command()
            .args(["list", "--complete"])
            .args(extra)
            .output()
            .expect("run");
        assert!(out.status.success());
        text(&out.stdout)
    };
    assert_eq!(complete(&[]), "acme/app\napp\nlib\nother/lib\n");
    assert_eq!(complete(&["--adopted"]), "lib\nother/lib\n");
}

#[test]
fn forget_reaches_a_repo_adopted_on_a_domain_no_longer_configured() {
    let env = TestEnv::new();
    env.config_domains(&["github.com", "ghe.example.com"]);
    env.seed_cache(&["github.com/acme/app"], false);
    readable(&env, "ghe.example.com", "corp/int", "corp/int");
    assert!(browse(&env, "ghe.example.com/corp/int").status.success());
    env.config_domains(&["github.com"]);

    let out = env
        .command()
        .args(["forget", "corp/int"])
        .output()
        .expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "Forgot ghe.example.com/corp/int\n");
}
