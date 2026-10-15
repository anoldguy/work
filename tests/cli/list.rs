//! `work list`.

use crate::support::{RepoSeed, TestEnv};

fn lines(out: &std::process::Output) -> Vec<String> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_owned)
        .collect()
}

fn assert_silent(out: &std::process::Output) {
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
}

fn seeded() -> TestEnv {
    let env = TestEnv::new();
    env.seed_cache(&[
        RepoSeed::new("github.com/acme/zeta"),
        RepoSeed::new("github.com/acme/alpha"),
        RepoSeed::new("github.com/other/alpha"),
    ]);
    env
}

#[test]
fn no_query_lists_every_slug_sorted() {
    let env = seeded();
    let out = env.command().arg("list").output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        lines(&out),
        [
            "github.com/acme/alpha",
            "github.com/acme/zeta",
            "github.com/other/alpha"
        ]
    );
}

#[test]
fn query_prints_ranked_results() {
    let env = seeded();
    let out = env.command().args(["list", "zeta"]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(lines(&out)[0], "github.com/acme/zeta");
}

#[test]
fn query_with_no_match_exits_zero_with_empty_output() {
    let env = seeded();
    let out = env.command().args(["list", "nomatchxyz"]).output().unwrap();
    assert!(out.status.success());
    assert_silent(&out);
}

#[test]
fn filter_is_substring_only() {
    let env = seeded();
    let out = env
        .command()
        .args(["list", "--filter", "other/"])
        .output()
        .unwrap();
    assert_eq!(lines(&out), ["github.com/other/alpha"]);
    let out = env
        .command()
        .args(["list", "--filter", "zt"])
        .output()
        .unwrap();
    assert_eq!(lines(&out), Vec::<String>::new());
}

#[test]
fn complete_prints_deduplicated_names_and_org_names() {
    let env = seeded();
    let out = env.command().args(["list", "--complete"]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        lines(&out),
        ["acme/alpha", "acme/zeta", "alpha", "other/alpha", "zeta"]
    );
}

#[test]
fn complete_is_silent_with_broken_config() {
    let env = seeded();
    env.write_config_raw("not = [valid");
    let out = env.command().args(["list", "--complete"]).output().unwrap();
    assert!(out.status.success());
    assert_silent(&out);
}

#[test]
fn complete_is_silent_with_broken_cache() {
    let env = seeded();
    std::fs::write(env.cache_path(), "garbage").unwrap();
    let out = env.command().args(["list", "--complete"]).output().unwrap();
    assert!(out.status.success());
    assert_silent(&out);
}

#[test]
fn complete_without_cache_is_silent() {
    let env = TestEnv::new();
    let out = env.command().args(["list", "--complete"]).output().unwrap();
    assert!(out.status.success());
    assert_silent(&out);
}

#[test]
fn stale_cache_is_refreshed_before_listing() {
    use crate::support::{DomainCfg, repo_line};
    let env = TestEnv::new();
    env.config()
        .domain(DomainCfg::new("github.com").default_domain())
        .write();
    env.cache()
        .repos(&[RepoSeed::new("github.com/acme/old")])
        .stale()
        .write();
    env.gh.set_api("github.com", "user", &["me".to_owned()]);
    env.gh.set_api("github.com", "user/orgs", &[]);
    env.gh.set_api(
        "github.com",
        "user/repos",
        &[repo_line("github.com", "me", "fresh")],
    );
    let out = env.command().arg("list").output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(lines(&out).contains(&"github.com/me/fresh".to_owned()));
    let cache = std::fs::read_to_string(env.cache_path()).unwrap();
    assert!(cache.contains("me/fresh"));
}

#[test]
fn broken_pipe_exits_quietly() {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    let env = TestEnv::new();
    let repos: Vec<RepoSeed> = (0..20_000)
        .map(|i| RepoSeed::new(&format!("github.com/acme/repo-{i:05}")))
        .collect();
    env.seed_cache(&repos);
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin!("work"))
        .env_clear()
        .envs(env.vars())
        .current_dir(env.home())
        .arg("list")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut first = String::new();
    BufReader::new(stdout).read_line(&mut first).unwrap();
    assert_eq!(first.trim_end(), "github.com/acme/repo-00000");
    // The reader (and its pipe end) is dropped here.
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{:?}", out.status);
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
}
