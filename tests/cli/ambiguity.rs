//! An ambiguous query without a terminal is an error that lists the matches.

use crate::support::{TestEnv, text};

#[test]
fn ambiguous_query_without_a_tty_lists_every_candidate() {
    let env = TestEnv::new();
    env.config_domains(&["github.com"]);
    let slugs = [
        "github.com/acme/foo-api",
        "github.com/acme/foo-web",
        "github.com/other/foo-cli",
    ];
    env.seed_cache(&slugs, false);

    let out = env.command().args(["clone", "foo"]).output().expect("run");
    assert!(!out.status.success());
    assert_eq!(text(&out.stdout), "");
    let stderr = text(&out.stderr);
    let mut listed: Vec<&str> = stderr
        .lines()
        .skip(1)
        .map(|l| l.strip_prefix("  - ").unwrap_or_else(|| panic!("{stderr}")))
        .collect();
    listed.sort_unstable();
    assert_eq!(listed, slugs);
    assert!(!env.work_dir().join("acme").exists());
}
