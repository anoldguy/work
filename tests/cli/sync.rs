//! `pull` and `sync`: every outcome line, exit codes, cache independence.

use std::fs;
use std::path::PathBuf;

use predicates::prelude::*;

use crate::support::{DomainCfg, Origin, RepoSeed, TestEnv, git};

const SLUG: &str = "github.com/acme/foo";

/// A configured env with `acme/foo` cached, cloned from a local origin.
fn cloned(env: &TestEnv) -> (Origin, PathBuf) {
    env.config()
        .domain(DomainCfg::new("github.com").default_domain())
        .write();
    let origin = env.origin("acme", "foo");
    env.seed_cache(&[RepoSeed::new(SLUG).origin(&origin)]);
    let path = env.clone_into_work_dir(&origin, SLUG);
    (origin, path)
}

fn pull(env: &TestEnv) -> assert_cmd::assert::Assert {
    env.command().args(["pull", "foo"]).assert()
}

#[test]
fn pull_reports_the_commits_it_fast_forwarded() {
    let env = TestEnv::new();
    let (origin, path) = cloned(&env);
    origin.commit_file("a.txt", "a\n", "one");
    let head = origin.commit_file("b.txt", "b\n", "two");
    pull(&env)
        .success()
        .stdout("acme/foo: pulled (2 commits)\n");
    assert_eq!(git(&path, &["rev-parse", "HEAD"]), head);
}

#[test]
fn pull_reports_up_to_date() {
    let env = TestEnv::new();
    cloned(&env);
    pull(&env).success().stdout("acme/foo: up to date\n");
}

#[test]
fn pull_with_a_dirty_worktree_only_fetches() {
    let env = TestEnv::new();
    let (origin, path) = cloned(&env);
    origin.commit_file("a.txt", "a\n", "one");
    fs::write(path.join("README.md"), "edited\n").expect("write");
    pull(&env)
        .success()
        .stdout("acme/foo: fetched (dirty worktree)\n");
    assert_eq!(
        fs::read_to_string(path.join("README.md")).expect("read"),
        "edited\n"
    );
}

#[test]
fn pull_with_untracked_files_counts_as_dirty() {
    let env = TestEnv::new();
    let (_origin, path) = cloned(&env);
    fs::write(path.join("new.txt"), "x\n").expect("write");
    pull(&env)
        .success()
        .stdout("acme/foo: fetched (dirty worktree)\n");
}

#[test]
fn pull_of_a_diverged_branch_only_fetches() {
    let env = TestEnv::new();
    let (origin, path) = cloned(&env);
    origin.commit_file("a.txt", "a\n", "remote");
    fs::write(path.join("l.txt"), "l\n").expect("write");
    git(&path, &["add", "l.txt"]);
    git(&path, &["commit", "-m", "local"]);
    pull(&env)
        .success()
        .stdout("acme/foo: fetched (diverged)\n");
}

#[test]
fn pull_with_a_detached_head_only_fetches() {
    let env = TestEnv::new();
    let (_origin, path) = cloned(&env);
    git(&path, &["checkout", "--detach"]);
    pull(&env)
        .success()
        .stdout("acme/foo: fetched (detached HEAD)\n");
}

#[test]
fn pull_of_a_branch_without_an_upstream_only_fetches() {
    let env = TestEnv::new();
    let (_origin, path) = cloned(&env);
    git(&path, &["checkout", "-b", "local-only"]);
    pull(&env)
        .success()
        .stdout("acme/foo: fetched (no upstream)\n");
}

#[test]
fn pull_follows_a_differently_named_upstream() {
    let env = TestEnv::new();
    let (origin, path) = cloned(&env);
    git(origin.bare_path(), &["branch", "release", "main"]);
    git(&path, &["fetch", "origin"]);
    git(
        &path,
        &["checkout", "-b", "work", "--track", "origin/release"],
    );
    let head = origin.commit_file("a.txt", "a\n", "one");
    git(origin.bare_path(), &["branch", "-f", "release", &head]);
    pull(&env)
        .success()
        .stdout("acme/foo: pulled (1 commits)\n");
}

#[test]
fn pull_failure_is_a_one_line_error_and_does_not_record_frecency() {
    let env = TestEnv::new();
    let (origin, _path) = cloned(&env);
    fs::remove_dir_all(origin.bare_path()).expect("remove origin");
    pull(&env)
        .code(1)
        .stdout("")
        .stderr(predicate::str::starts_with(
            "work: could not pull acme/foo: ",
        ))
        .stderr(predicate::str::ends_with("\n"));
    let cache = fs::read_to_string(env.cache_path()).expect("cache");
    assert!(!cache.contains("access_count"), "{cache}");
}

#[test]
fn pull_records_frecency_on_success() {
    let env = TestEnv::new();
    cloned(&env);
    pull(&env).success();
    let cache = fs::read_to_string(env.cache_path()).expect("cache");
    assert!(cache.contains("access_count"), "{cache}");
}

#[test]
fn pull_of_an_uncloned_repo_is_an_error() {
    let env = TestEnv::new();
    let (_origin, path) = cloned(&env);
    fs::remove_dir_all(path).expect("remove clone");
    pull(&env)
        .code(1)
        .stdout("")
        .stderr("work: acme/foo is not cloned (run 'work clone acme/foo')\n");
}

#[test]
fn pull_without_a_cache_is_a_one_line_error() {
    let env = TestEnv::new();
    env.config()
        .domain(DomainCfg::new("github.com").default_domain())
        .write();
    pull(&env)
        .code(1)
        .stdout("")
        .stderr("work: no usable repository cache (run 'work update')\n");
}

#[test]
fn sync_prints_one_sorted_line_per_repo_and_ignores_the_cache() {
    let env = TestEnv::new();
    env.config()
        .domain(DomainCfg::new("github.com").default_domain())
        .write();
    let behind = env.origin("acme", "behind");
    let current = env.origin("acme", "current");
    let local = env.origin("zed", "local");
    env.clone_into_work_dir(&behind, "github.com/acme/behind");
    env.clone_into_work_dir(&current, "github.com/acme/current");
    let local_path = env.clone_into_work_dir(&local, "github.com/zed/local");
    git(&local_path, &["checkout", "-b", "scratch"]);
    behind.commit_file("a.txt", "a\n", "one");
    let assert = env.command().arg("sync").assert().success().stderr("");
    let out = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3, "{out}");
    assert!(lines[0].ends_with("behind: pulled (1 commits)"), "{out}");
    assert!(lines[1].ends_with("current: up to date"), "{out}");
    assert!(lines[2].ends_with("local: fetched (no upstream)"), "{out}");
}

#[test]
fn sync_never_touches_a_corrupt_cache() {
    let env = TestEnv::new();
    env.config()
        .domain(DomainCfg::new("github.com").default_domain())
        .write();
    let origin = env.origin("acme", "foo");
    env.clone_into_work_dir(&origin, SLUG);
    let cache = env.cache_path();
    fs::create_dir_all(cache.parent().expect("parent")).expect("mkdir");
    fs::write(&cache, "{ not json").expect("write");
    env.command().arg("sync").assert().success().stderr("");
    assert_eq!(fs::read_to_string(&cache).expect("read"), "{ not json");
    assert!(!cache.with_extension("json.corrupt").exists());
}

#[test]
fn sync_does_not_descend_into_a_repo_to_find_nested_repos() {
    let env = TestEnv::new();
    env.config()
        .domain(DomainCfg::new("github.com").default_domain())
        .write();
    let outer = env.origin("acme", "foo");
    let inner = env.origin("acme", "inner");
    let path = env.clone_into_work_dir(&outer, SLUG);
    inner.clone_to(&path.join("vendor").join("inner"));
    let assert = env.command().arg("sync").assert().success();
    let out = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");
    assert_eq!(out.lines().count(), 1, "{out}");
    assert!(out.starts_with("acme/foo: "), "{out}");
}

#[test]
fn sync_error_prints_every_line_then_exits_1_naming_the_failures() {
    let env = TestEnv::new();
    env.config()
        .domain(DomainCfg::new("github.com").default_domain())
        .write();
    let good = env.origin("acme", "good");
    let bad = env.origin("acme", "bad");
    env.clone_into_work_dir(&good, "github.com/acme/good");
    let bad_path = env.clone_into_work_dir(&bad, "github.com/acme/bad");
    fs::remove_dir_all(bad.bare_path()).expect("remove origin");
    let rel = bad_path
        .strip_prefix(env.work_dir())
        .expect("under work dir")
        .display()
        .to_string();
    env.command()
        .args(["sync", "--jobs", "2"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("good: up to date\n"))
        .stdout(predicate::str::contains(format!("{rel}: error: ")))
        .stderr(predicate::str::starts_with(
            "work: 1 of 2 repositories failed to sync\n",
        ))
        .stderr(predicate::str::contains(format!("  - {rel}\n")));
}

#[test]
fn sync_rejects_zero_jobs() {
    let env = TestEnv::new();
    env.command()
        .args(["sync", "--jobs", "0"])
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("--jobs"));
}

#[test]
fn sync_with_no_repos_prints_nothing_and_succeeds() {
    let env = TestEnv::new();
    fs::create_dir_all(env.work_dir()).expect("mkdir");
    env.command().arg("sync").assert().success().stdout("");
}
