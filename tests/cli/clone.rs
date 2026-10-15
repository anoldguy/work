//! Auto-clone: the target path, the scoped credential helper, and cleanup
//! after a failed clone.

use crate::support::{TestEnv, dirs_under, line, text};

#[test]
fn clone_places_repos_by_domain_and_authenticates_through_gh() {
    let env = TestEnv::new();
    env.config_domains(&["github.com", "ghe.example.com"]);
    env.seed_cache(&["github.com/acme/app", "ghe.example.com/corp/tool"], false);
    let app = env.remote("github.com/acme/app");
    let tool = env.remote("ghe.example.com/corp/tool");
    assert_ne!(app.head(), tool.head());

    for (query, path, remote, host) in [
        ("app", env.work_dir().join("acme/app"), &app, "github.com"),
        (
            "tool",
            env.work_dir().join("ghe.example.com/corp/tool"),
            &tool,
            "ghe.example.com",
        ),
    ] {
        let out = env.command().args(["clone", query]).output().expect("run");
        assert!(out.status.success(), "{}", text(&out.stderr));
        assert_eq!(text(&out.stdout), line(&path));
        assert_eq!(env.git(&path, &["rev-parse", "HEAD"]), remote.head());
        let helper = format!("credential.https://{host}.helper=!gh auth git-credential");
        let clone = env
            .calls("git")
            .into_iter()
            .find(|call| call.contains(" clone ") && call.contains(query))
            .expect("git clone ran");
        assert!(clone.contains("-c credential.helper= "), "{clone}");
        assert!(clone.contains("-c protocol.ext.allow=never "), "{clone}");
        assert!(clone.contains(&helper), "{clone}");
    }
}

#[test]
fn failed_clone_leaves_no_new_directories() {
    let env = TestEnv::new();
    env.config_domains(&["github.com"]);
    env.seed_cache(&["github.com/acme/gone"], false);
    let before = dirs_under(env.home().as_path());

    let out = env.command().args(["clone", "gone"]).output().expect("run");
    assert!(!out.status.success());
    assert_eq!(text(&out.stdout), "");
    assert_eq!(
        text(&out.stderr).lines().count(),
        1,
        "{}",
        text(&out.stderr)
    );
    assert_eq!(dirs_under(env.home().as_path()), before);
    assert!(!env.frecency_path().exists());
}
