//! `pull` and `sync`: every outcome line, exit codes, which repos sync, auth scoping.

use std::fs;
use std::path::{Path, PathBuf};

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
    pull(&env).success().stdout("acme/foo: pulled (1 commit)\n");
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

/// Writes a config with `github.com` as the default domain.
fn github_default(env: &TestEnv) {
    env.config()
        .domain(DomainCfg::new("github.com").default_domain())
        .write();
}

fn sync(env: &TestEnv) -> assert_cmd::assert::Assert {
    env.command().arg("sync").assert()
}

/// What `origin/main` points at in the clone at `path` (moves only on fetch).
fn fetched_main(path: &Path) -> String {
    git(path, &["rev-parse", "origin/main"])
}

/// The `GIT_CONFIG_KEY_n` list of the one `git fetch` run in `path`.
fn fetch_keys(log: &str, path: &Path) -> String {
    let marker = format!("-C {} fetch", path.display());
    let lines: Vec<&str> = log.lines().filter(|l| l.contains(&marker)).collect();
    assert_eq!(lines.len(), 1, "{marker}\n{log}");
    lines[0]
        .split_once(" |")
        .map(|(_, keys)| keys.to_owned())
        .expect("separator")
}

#[test]
fn sync_prints_one_sorted_line_per_cached_and_cloned_repo() {
    let env = TestEnv::new();
    github_default(&env);
    let behind = env.origin("acme", "behind");
    let current = env.origin("acme", "current");
    let local = env.origin("zed", "local");
    env.seed_cache(&[
        RepoSeed::new("github.com/zed/local").origin(&local),
        RepoSeed::new("github.com/acme/current").origin(&current),
        RepoSeed::new("github.com/acme/behind").origin(&behind),
    ]);
    env.clone_into_work_dir(&behind, "github.com/acme/behind");
    env.clone_into_work_dir(&current, "github.com/acme/current");
    let local_path = env.clone_into_work_dir(&local, "github.com/zed/local");
    git(&local_path, &["checkout", "-b", "scratch"]);
    behind.commit_file("a.txt", "a\n", "one");
    sync(&env).success().stderr("").stdout(
        "acme/behind: pulled (1 commit)\n\
         acme/current: up to date\n\
         zed/local: fetched (no upstream)\n",
    );
}

#[test]
fn sync_ignores_repos_in_the_work_dir_that_are_not_cached() {
    let env = TestEnv::new();
    let (_origin, _path) = cloned(&env);
    let stray = env.origin("acme", "stray");
    let stray_path = env.clone_into_work_dir(&stray, "github.com/acme/stray");
    let nested = env.origin("acme", "nested");
    let nested_path = env.repo_path(SLUG).join("vendor").join("nested");
    nested.clone_to(&nested_path);
    let stray_before = fetched_main(&stray_path);
    let nested_before = fetched_main(&nested_path);
    stray.commit_file("a.txt", "a\n", "one");
    nested.commit_file("a.txt", "a\n", "one");
    // The nested clone is untracked content, so `acme/foo` is dirty.
    sync(&env)
        .success()
        .stderr("")
        .stdout("acme/foo: fetched (dirty worktree)\n");
    assert_eq!(fetched_main(&stray_path), stray_before);
    assert_eq!(fetched_main(&nested_path), nested_before);
}

#[test]
fn sync_ignores_cached_repos_that_are_not_cloned() {
    let env = TestEnv::new();
    github_default(&env);
    let foo = env.origin("acme", "foo");
    let bar = env.origin("acme", "bar");
    env.seed_cache(&[
        RepoSeed::new(SLUG).origin(&foo),
        RepoSeed::new("github.com/acme/bar").origin(&bar),
    ]);
    env.clone_into_work_dir(&foo, SLUG);
    // A directory without `.git` at the expected path is not a clone.
    fs::create_dir_all(env.repo_path("github.com/acme/bar")).expect("mkdir");
    sync(&env)
        .success()
        .stderr("")
        .stdout("acme/foo: up to date\n");
}

#[test]
fn sync_without_a_cache_is_a_one_line_error_and_fetches_nothing() {
    let env = TestEnv::new();
    github_default(&env);
    let origin = env.origin("acme", "foo");
    let path = env.clone_into_work_dir(&origin, SLUG);
    let before = fetched_main(&path);
    origin.commit_file("a.txt", "a\n", "one");
    sync(&env)
        .code(1)
        .stdout("")
        .stderr("work: no usable repository cache (run 'work update')\n");
    assert_eq!(fetched_main(&path), before);
}

#[test]
fn sync_neither_writes_the_cache_nor_records_frecency() {
    let env = TestEnv::new();
    cloned(&env);
    let before = fs::read_to_string(env.cache_path()).expect("cache");
    sync(&env).success();
    assert_eq!(fs::read_to_string(env.cache_path()).expect("cache"), before);
}

#[test]
fn sync_scopes_each_token_to_the_repos_cached_domain() {
    let env = TestEnv::new();
    env.config()
        .domain(DomainCfg::new("github.com").default_domain())
        .domain(DomainCfg::new("ghe.example.com"))
        .write();
    let foo = env.origin("acme", "foo");
    let bar = env.origin("acme", "bar");
    env.seed_cache(&[
        RepoSeed::new(SLUG).origin(&foo),
        RepoSeed::new("ghe.example.com/acme/bar").origin(&bar),
    ]);
    let foo_path = env.clone_into_work_dir(&foo, SLUG);
    let bar_path = env.clone_into_work_dir(&bar, "ghe.example.com/acme/bar");
    // `origin` points at neither domain: the cache, not the remote, decides.
    git(&bar_path, &["remote", "set-url", "origin", &foo.url()]);
    let log = env.record_git_config_keys();
    sync(&env).success().stdout(
        "acme/foo: up to date\n\
         ghe.example.com/acme/bar: up to date\n",
    );
    let log = fs::read_to_string(log).expect("git log");
    let foo_keys = fetch_keys(&log, &foo_path);
    let bar_keys = fetch_keys(&log, &bar_path);
    assert!(
        foo_keys.contains("http.https://github.com/.extraHeader"),
        "{foo_keys}"
    );
    assert!(!foo_keys.contains("ghe.example.com"), "{foo_keys}");
    assert!(
        bar_keys.contains("http.https://ghe.example.com/.extraHeader"),
        "{bar_keys}"
    );
    assert!(!bar_keys.contains("github.com/"), "{bar_keys}");
    assert!(!log.contains("test-token"), "{log}");
}

#[test]
fn sync_resolves_gh_tokens_per_cached_domain_and_fetches_the_rest_unauthenticated() {
    let env = TestEnv::new().token(None);
    env.gh.set_token("github.com", "gho_gh_token");
    env.config()
        .domain(DomainCfg::new("github.com").default_domain())
        .domain(DomainCfg::new("ghe.example.com"))
        .write();
    let foo = env.origin("acme", "foo");
    let bar = env.origin("acme", "bar");
    env.seed_cache(&[
        RepoSeed::new(SLUG).origin(&foo),
        RepoSeed::new("ghe.example.com/acme/bar").origin(&bar),
    ]);
    let foo_path = env.clone_into_work_dir(&foo, SLUG);
    let bar_path = env.clone_into_work_dir(&bar, "ghe.example.com/acme/bar");
    let log = env.record_git_config_keys();
    sync(&env).success();
    let log = fs::read_to_string(log).expect("git log");
    assert!(
        fetch_keys(&log, &foo_path).contains("http.https://github.com/.extraHeader"),
        "{log}"
    );
    assert!(
        !fetch_keys(&log, &bar_path).contains("extraHeader"),
        "{log}"
    );
}

#[test]
fn sync_with_an_invalid_env_token_fails_on_one_line_before_fetching() {
    let env = TestEnv::new().token(Some("bad token"));
    let (origin, path) = cloned(&env);
    let before = fetched_main(&path);
    origin.commit_file("a.txt", "a\n", "one");
    sync(&env).code(1).stdout("").stderr(
        "work: GITHUB_TOKEN is set but invalid \
         (it must not contain whitespace or control characters)\n",
    );
    assert_eq!(fetched_main(&path), before);
}

#[test]
fn sync_error_prints_every_line_then_exits_1_naming_the_failures() {
    let env = TestEnv::new();
    github_default(&env);
    let good = env.origin("acme", "good");
    let bad = env.origin("acme", "bad");
    env.seed_cache(&[
        RepoSeed::new("github.com/acme/good").origin(&good),
        RepoSeed::new("github.com/acme/bad").origin(&bad),
    ]);
    env.clone_into_work_dir(&good, "github.com/acme/good");
    env.clone_into_work_dir(&bad, "github.com/acme/bad");
    fs::remove_dir_all(bad.bare_path()).expect("remove origin");
    env.command()
        .args(["sync", "--jobs", "2"])
        .assert()
        .code(1)
        .stdout(predicate::str::starts_with("acme/bad: error: "))
        .stdout(predicate::str::ends_with("\nacme/good: up to date\n"))
        .stderr("work: 1 of 2 repositories failed to sync\n  - acme/bad\n");
}

#[test]
fn sync_rejects_zero_jobs() {
    let env = TestEnv::new();
    env.command()
        .args(["sync", "--jobs", "0"])
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("--jobs"))
        .stderr(predicate::str::contains("must be at least 1"))
        .stderr(predicate::str::contains("non-zero type").not());
}

#[test]
fn sync_with_nothing_cloned_says_so_on_stderr_and_succeeds() {
    let env = TestEnv::new();
    github_default(&env);
    env.seed_cache(&[RepoSeed::new(SLUG)]);
    sync(&env).success().stdout("").stderr(format!(
        "no cached repositories are cloned under {}\n",
        env.work_dir().display()
    ));
}

#[test]
fn sync_with_a_missing_work_dir_says_nothing_is_cloned_and_succeeds() {
    let env = TestEnv::new();
    cloned(&env);
    fs::remove_dir_all(env.work_dir()).expect("remove work dir");
    sync(&env).success().stdout("").stderr(format!(
        "no cached repositories are cloned under {}\n",
        env.work_dir().display()
    ));
}

#[test]
fn sync_with_a_corrupt_cache_errors_and_leaves_the_file_alone() {
    let env = TestEnv::new();
    github_default(&env);
    let origin = env.origin("acme", "foo");
    let path = env.clone_into_work_dir(&origin, SLUG);
    let before = fetched_main(&path);
    origin.commit_file("a.txt", "a\n", "one");
    let cache = env.cache_path();
    fs::create_dir_all(cache.parent().expect("parent")).expect("mkdir");
    fs::write(&cache, "{ not json").expect("write");
    sync(&env)
        .code(1)
        .stdout("")
        .stderr(predicate::str::ends_with(
            "work: no usable repository cache (run 'work update')\n",
        ));
    assert_eq!(fs::read_to_string(&cache).expect("read"), "{ not json");
    let siblings: Vec<_> = fs::read_dir(cache.parent().expect("parent"))
        .expect("read cache dir")
        .map(|e| e.expect("entry").file_name())
        .collect();
    assert!(
        siblings
            .iter()
            .all(|n| !n.to_string_lossy().contains("corrupt")),
        "{siblings:?}"
    );
    assert_eq!(fetched_main(&path), before);
}
