//! Ambiguous queries: the error without a terminal, `--pick`, and query
//! frecency settling the answer.

use std::fs;

use serde_json::Value;

use crate::support::{TestEnv, text};

#[test]
fn ambiguous_query_without_a_tty_lists_every_candidate_in_rank_order() {
    let env = TestEnv::new();
    env.config_domains(&["github.com"]);
    env.seed_cache(
        &[
            "github.com/acme/foo-api",
            "github.com/acme/foo-web",
            "github.com/other/foo-cli",
        ],
        false,
    );
    env.seed_frecency(&[("github.com/other/foo-cli", 9)]);

    let out = env.command().args(["clone", "foo"]).output().expect("run");
    assert!(!out.status.success());
    assert_eq!(text(&out.stdout), "");
    let stderr = text(&out.stderr);
    let listed: Vec<&str> = stderr
        .lines()
        .skip(1)
        .map(|l| l.strip_prefix("  - ").unwrap_or_else(|| panic!("{stderr}")))
        .collect();
    assert_eq!(
        listed,
        [
            "github.com/other/foo-cli",
            "github.com/acme/foo-api",
            "github.com/acme/foo-web",
        ]
    );
    assert!(!env.work_dir().join("acme").exists());
}

#[test]
fn one_exact_match_among_several_resolves_without_asking() {
    let env = TestEnv::new();
    env.config_domains(&["github.com"]);
    env.seed_cache(&["github.com/acme/foo", "github.com/acme/foobar"], false);
    env.remote("github.com/acme/foo");

    let out = env.command().args(["clone", "foo"]).output().expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(env.work_dir().join("acme/foo/.git").is_dir());
}

#[test]
fn pick_without_a_tty_fails_even_with_an_exact_match() {
    let env = TestEnv::new();
    env.config_domains(&["github.com"]);
    env.seed_cache(&["github.com/acme/foo", "github.com/acme/foobar"], false);
    env.remote("github.com/acme/foo");

    let out = env
        .command()
        .args(["clone", "foo", "--pick"])
        .output()
        .expect("run");
    assert!(!out.status.success());
    assert_eq!(text(&out.stdout), "");
    assert!(!env.work_dir().join("acme").exists());
}

#[test]
fn a_learned_query_resolves_without_a_tty_and_counts_the_pick() {
    let env = TestEnv::new();
    env.config_domains(&["github.com"]);
    env.seed_cache(
        &["github.com/acme/foo-api", "github.com/acme/foo-web"],
        false,
    );
    env.seed_history(&[], &[("foo", "github.com/acme/foo-api", 3)]);
    env.remote("github.com/acme/foo-api");

    let out = env.command().args(["clone", "foo"]).output().expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(env.work_dir().join("acme/foo-api/.git").is_dir());
    assert!(!env.work_dir().join("acme/foo-web").exists());
    let history: Value =
        serde_json::from_slice(&fs::read(env.frecency_path()).expect("frecency")).expect("json");
    assert_eq!(
        history["query_frecency"]["foo"]["github.com/acme/foo-api"]["access_count"], 4,
        "{history}"
    );
}
