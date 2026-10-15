//! `frecency.json` outlives the listing and is never lost.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use crate::support::{TestEnv, text};

fn list(env: &TestEnv, query: &str) -> Vec<String> {
    let out = env.command().args(["list", query]).output().expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    text(&out.stdout).lines().map(str::to_owned).collect()
}

fn update(env: &TestEnv) {
    let out = env.command().arg("update").output().expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
}

#[test]
fn frecency_survives_a_refresh_that_drops_the_repo() {
    let env = TestEnv::new();
    env.config_domains(&["github.com"]);
    env.gh_domain("github.com", &["acme"], &["acme/foo-a", "acme/foo-b"]);
    update(&env);
    assert_eq!(list(&env, "foo")[0], "github.com/acme/foo-a");
    env.seed_frecency(&[("github.com/acme/foo-b", 9)]);
    assert_eq!(list(&env, "foo")[0], "github.com/acme/foo-b");
    let history = fs::read(env.frecency_path()).expect("frecency");

    env.gh_domain("github.com", &["acme"], &["acme/foo-a"]);
    update(&env);
    assert_eq!(list(&env, "foo"), ["github.com/acme/foo-a"]);
    assert_eq!(fs::read(env.frecency_path()).expect("frecency"), history);

    env.gh_domain("github.com", &["acme"], &["acme/foo-a", "acme/foo-b"]);
    update(&env);
    assert_eq!(list(&env, "foo")[0], "github.com/acme/foo-b");
}

/// Seeds `acme/app` with a remote and runs `work clone app`, which records
/// an access.
fn clone_app(env: &TestEnv) -> String {
    env.config_domains(&["github.com"]);
    env.seed_cache(&["github.com/acme/app"], false);
    env.remote("github.com/acme/app");
    let out = env.command().args(["clone", "app"]).output().expect("run");
    let stderr = text(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    stderr
}

#[test]
fn a_corrupt_file_is_set_aside_with_a_warning() {
    let env = TestEnv::new();
    fs::create_dir_all(env.frecency_path().parent().expect("dir")).expect("state dir");
    fs::write(env.frecency_path(), "{not json").expect("write");

    let stderr = clone_app(&env);
    assert!(stderr.contains("frecency.json"), "{stderr}");
    let corrupt = env.frecency_path().with_extension("json.corrupt");
    assert_eq!(fs::read_to_string(corrupt).expect("renamed"), "{not json");
    let history = fs::read_to_string(env.frecency_path()).expect("new file");
    assert!(history.contains("github.com/acme/app"), "{history}");
}

#[test]
fn an_unknown_version_is_never_overwritten() {
    let env = TestEnv::new();
    let future = r#"{"version": 99, "frecency": {}}"#;
    fs::create_dir_all(env.frecency_path().parent().expect("dir")).expect("state dir");
    fs::write(env.frecency_path(), future).expect("write");

    let stderr = clone_app(&env);
    assert!(stderr.contains("frecency"), "{stderr}");
    assert_eq!(
        fs::read_to_string(env.frecency_path()).expect("kept"),
        future
    );
}

#[test]
fn an_unreadable_file_is_left_alone() {
    let env = TestEnv::new();
    env.seed_frecency(&[("github.com/acme/other", 2)]);
    let before = fs::read(env.frecency_path()).expect("frecency");
    let set_mode = |mode| {
        fs::set_permissions(env.frecency_path(), fs::Permissions::from_mode(mode)).expect("chmod");
    };
    set_mode(0o000);
    if fs::read(env.frecency_path()).is_ok() {
        eprintln!("running with permission to read a mode-000 file; skipping");
        return;
    }

    let stderr = clone_app(&env);
    set_mode(0o644);
    assert!(stderr.contains("frecency"), "{stderr}");
    assert_eq!(fs::read(env.frecency_path()).expect("kept"), before);
    assert!(!env.frecency_path().with_extension("json.corrupt").exists());
}
