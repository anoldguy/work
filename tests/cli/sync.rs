//! `work pull` and `work sync` with real git.

use std::fs;

use crate::support::{TestEnv, text};

#[test]
fn sync_reports_every_outcome_and_fails_listing_the_errors() {
    let env = TestEnv::new();
    env.config_domains(&["github.com"]);
    let names = [
        "pulled", "current", "dirty", "diverged", "detached", "local", "broken",
    ];
    let slugs: Vec<String> = names
        .iter()
        .map(|n| format!("github.com/acme/{n}"))
        .collect();
    let slug_refs: Vec<&str> = slugs.iter().map(String::as_str).collect();
    env.seed_cache(&slug_refs, false);
    let clone = |name: &str| {
        let remote = env.remote(&format!("github.com/acme/{name}"));
        let path = remote.clone_to(&env.work_dir().join("acme").join(name));
        (remote, path)
    };

    let (remote, pulled) = clone("pulled");
    remote.commit("a.txt", "a\n");
    let pulled_head = remote.commit("b.txt", "b\n");
    clone("current");
    let (remote, dirty) = clone("dirty");
    let dirty_head = env.git(&dirty, &["rev-parse", "HEAD"]);
    remote.commit("a.txt", "a\n");
    fs::write(dirty.join("README.md"), "edited\n").expect("edit");
    let (remote, diverged) = clone("diverged");
    remote.commit("a.txt", "remote\n");
    fs::write(diverged.join("l.txt"), "local\n").expect("write");
    env.git(&diverged, &["add", "l.txt"]);
    env.git(&diverged, &["commit", "-m", "local"]);
    let (_, detached) = clone("detached");
    env.git(&detached, &["checkout", "--detach"]);
    let (_, local) = clone("local");
    env.git(&local, &["checkout", "-b", "topic"]);
    let (remote, _) = clone("broken");
    fs::remove_dir_all(remote.bare()).expect("remove remote");

    let out = env.command().arg("sync").output().expect("run");
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stdout}{stderr}");
    let outcome = |name: &str| -> String {
        stdout
            .lines()
            .find(|l| l.contains(&format!("acme/{name}:")))
            .unwrap_or_else(|| panic!("no line for {name}: {stdout}"))
            .to_owned()
    };
    assert!(
        outcome("pulled").ends_with("pulled (2 commits)"),
        "{stdout}"
    );
    assert!(outcome("current").ends_with("up to date"), "{stdout}");
    assert!(
        outcome("dirty").ends_with("fetched (dirty worktree)"),
        "{stdout}"
    );
    assert!(
        outcome("diverged").ends_with("fetched (diverged)"),
        "{stdout}"
    );
    assert!(
        outcome("detached").ends_with("fetched (detached HEAD)"),
        "{stdout}"
    );
    assert!(
        outcome("local").ends_with("fetched (no upstream)"),
        "{stdout}"
    );
    assert!(outcome("broken").contains("error: "), "{stdout}");
    assert_eq!(stdout.lines().count(), names.len(), "{stdout}");

    assert_eq!(env.git(&pulled, &["rev-parse", "HEAD"]), pulled_head);
    assert_eq!(env.git(&dirty, &["rev-parse", "HEAD"]), dirty_head);
    assert_eq!(
        fs::read_to_string(dirty.join("README.md")).expect("read"),
        "edited\n"
    );
    let fetches: Vec<String> = env
        .calls("git")
        .into_iter()
        .filter(|call| call.contains(" fetch"))
        .collect();
    assert_eq!(fetches.len(), names.len(), "{fetches:?}");
    for fetch in &fetches {
        assert!(
            fetch.contains("credential.https://github.com.helper=!gh auth git-credential"),
            "{fetch}"
        );
    }
    let failed: Vec<&str> = stderr.lines().filter(|l| l.starts_with("  - ")).collect();
    assert_eq!(failed.len(), 1, "{stderr}");
    assert!(failed[0].contains("acme/broken"), "{stderr}");
}

#[test]
fn sync_succeeds_when_every_repo_is_up_to_date() {
    let env = TestEnv::new();
    env.config_domains(&["github.com"]);
    env.seed_cache(&["github.com/acme/a", "github.com/acme/b"], false);
    for name in ["a", "b"] {
        env.remote(&format!("github.com/acme/{name}"))
            .clone_to(&env.work_dir().join("acme").join(name));
    }

    let out = env.command().arg("sync").output().expect("run");
    let stdout = text(&out.stdout);
    assert!(out.status.success(), "{stdout}{}", text(&out.stderr));
    assert_eq!(stdout.lines().count(), 2, "{stdout}");
    assert!(
        stdout.lines().all(|l| l.ends_with("up to date")),
        "{stdout}"
    );
}

#[test]
fn pull_fast_forwards_one_cloned_repo() {
    let env = TestEnv::new();
    env.config_domains(&["github.com"]);
    env.seed_cache(&["github.com/acme/app"], false);
    let remote = env.remote("github.com/acme/app");
    let path = remote.clone_to(&env.work_dir().join("acme/app"));
    let head = remote.commit("a.txt", "a\n");

    let out = env.command().args(["pull", "app"]).output().expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "acme/app: pulled (1 commit)\n");
    assert_eq!(env.git(&path, &["rev-parse", "HEAD"]), head);
}

#[test]
fn pull_on_a_repo_that_is_not_cloned_fails_without_running_git() {
    let env = TestEnv::new();
    env.config_domains(&["github.com"]);
    env.seed_cache(&["github.com/acme/app"], false);
    env.remote("github.com/acme/app");
    fs::create_dir_all(env.work_dir().join("acme/app")).expect("empty dir");

    let out = env.command().args(["pull", "app"]).output().expect("run");
    assert!(!out.status.success());
    assert_eq!(text(&out.stdout), "");
    assert_eq!(
        text(&out.stderr).lines().count(),
        1,
        "{}",
        text(&out.stderr)
    );
    assert_eq!(env.calls("git"), Vec::<String>::new());
}
