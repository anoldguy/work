//! `cd`, `on`, `edit`, `clone`, `browse`, `open` and their `sh-*` forms
//! (W5.1), against a seeded cache and real local clones.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::Value;

use crate::support::{DomainCfg, Origin, RepoSeed, TestEnv};

const SLUG: &str = "github.com/acme/foo";

/// A test env whose config makes github.com the default domain and whose
/// fresh cache holds `acme/foo`, cloned from a real local origin.
fn env_with_foo() -> (TestEnv, Origin) {
    let env = TestEnv::new();
    env.config()
        .domain(DomainCfg::new("github.com").default_domain())
        .write();
    let origin = env.origin("acme", "foo");
    env.seed_cache(&[RepoSeed::new(SLUG).origin(&origin)]);
    (env, origin)
}

fn run(env: &TestEnv, args: &[&str]) -> Output {
    env.command().args(args).output().expect("run work")
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("utf-8 stdout")
}

fn stderr(out: &Output) -> String {
    String::from_utf8(out.stderr.clone()).expect("utf-8 stderr")
}

/// The cache file as JSON.
fn cache_json(env: &TestEnv) -> Value {
    serde_json::from_str(&fs::read_to_string(env.cache_path()).expect("read cache"))
        .expect("parse cache")
}

/// The recorded repo frecency count for `slug` (0 when absent).
fn access_count(env: &TestEnv, slug: &str) -> u64 {
    cache_json(env)["frecency"][slug]["access_count"]
        .as_u64()
        .unwrap_or(0)
}

fn path_str(path: &Path) -> &str {
    path.to_str().expect("utf-8 path")
}

/// Quotes like `shell::quote`: `'` as `'\''`, `\` as `'\\'`.
fn quoted(path: &Path) -> String {
    let inner: String = path_str(path)
        .chars()
        .map(|c| match c {
            '\'' => r"'\''".to_owned(),
            '\\' => r"'\\'".to_owned(),
            c => c.to_string(),
        })
        .collect();
    format!("'{inner}'")
}

#[test]
fn sh_cd_clones_and_prints_one_cd_line() {
    let (env, origin) = env_with_foo();
    let path = env.repo_path(SLUG);
    let out = run(&env, &["sh-cd", "foo"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), format!("cd {}\n", quoted(&path)));
    assert!(path.join(".git").is_dir());
    let head = crate::support::git(&path, &["rev-parse", "HEAD"]);
    assert_eq!(head, origin.head());
    assert_eq!(access_count(&env, SLUG), 1);
}

#[test]
fn sh_cd_on_an_existing_clone_records_another_access() {
    let (env, origin) = env_with_foo();
    env.clone_into_work_dir(&origin, SLUG);
    for expected in 1..=2 {
        let out = run(&env, &["sh-cd", "acme/foo"]);
        assert!(out.status.success(), "stderr: {}", stderr(&out));
        assert_eq!(access_count(&env, SLUG), expected);
    }
}

#[test]
fn sh_cd_without_a_query_goes_to_the_work_dir_and_creates_it() {
    let (env, _origin) = env_with_foo();
    fs::remove_dir_all(env.work_dir()).expect("remove work dir");
    let out = run(&env, &["sh-cd"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), format!("cd {}\n", quoted(&env.work_dir())));
    assert!(env.work_dir().is_dir());
    assert_eq!(access_count(&env, SLUG), 0);
}

#[test]
fn sh_on_prints_cd_and_the_editor_on_one_line() {
    let (env, _origin) = env_with_foo();
    let path = env.repo_path(SLUG);
    let out = run(&env, &["sh-on", "foo"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let q = quoted(&path);
    assert_eq!(
        stdout(&out),
        format!("cd {q} && {} {q}\n", path_str(&env.editor.path()))
    );
    assert!(path.join(".git").is_dir());
    // The script is only printed; the wrapper runs the editor.
    assert_eq!(env.editor.calls(), Vec::<Vec<String>>::new());
    assert_eq!(access_count(&env, SLUG), 1);
}

/// A work dir full of characters that need quoting.
fn quote_heavy_env() -> (TestEnv, PathBuf, Origin) {
    let env = TestEnv::new();
    let work_dir = env.home().join(r#"it's a \ "$dir" `x`"#);
    let toml_value = path_str(&work_dir)
        .replace('\\', r"\\")
        .replace('"', "\\\"");
    env.write_config_raw(&format!("work_dir = \"{toml_value}\"\n"));
    let origin = env.origin("acme", "foo");
    env.seed_cache(&[RepoSeed::new(SLUG).origin(&origin)]);
    (env, work_dir.join("acme/foo"), origin)
}

#[test]
fn sh_cd_and_sh_on_quote_hostile_paths_and_sh_reads_them_back() {
    let (env, path, _origin) = quote_heavy_env();
    let cd = stdout(&run(&env, &["sh-cd", "foo"]));
    assert_eq!(cd, format!("cd {}\n", quoted(&path)));
    let on = stdout(&run(&env, &["sh-on", "foo"]));
    assert_eq!(on.lines().count(), 1);
    assert!(path.join(".git").is_dir());

    // Evaluating the script in a real POSIX shell lands in the directory.
    let shown = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(format!("{}; pwd -P", cd.trim_end()))
        .output()
        .expect("run sh");
    assert!(shown.status.success());
    let canonical = path.canonicalize().expect("canonical");
    assert_eq!(stdout(&shown), format!("{}\n", path_str(&canonical)));
}

#[test]
fn plain_cd_and_on_name_work_activate_without_resolving() {
    let env = TestEnv::new();
    for args in [&["cd", "foo"][..], &["cd"], &["on", "foo"]] {
        let out = env
            .command()
            .args(args)
            .env("SHELL", "/usr/bin/fish")
            .output()
            .expect("run");
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        assert_eq!(stdout(&out), "");
        assert_eq!(
            stderr(&out),
            "work: shell integration is not active (add 'work activate fish | source' to \
             ~/.config/fish/config.fish, then restart your shell)\n"
        );
    }
    // Nothing was resolved: no cache was fetched or written.
    assert_eq!(env.api.requests().len(), 0);
    assert!(!env.cache_path().exists());
}

#[test]
fn plain_cd_hint_defaults_to_bash() {
    let env = TestEnv::new();
    let out = run(&env, &["cd", "foo"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains(r#"'eval "$(work activate bash)"' to ~/.bashrc"#),
        "{}",
        stderr(&out)
    );
}

#[test]
fn clone_clones_and_prints_the_path_then_is_idempotent() {
    let (env, origin) = env_with_foo();
    let path = env.repo_path(SLUG);
    let out = run(&env, &["clone", "foo"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), format!("{}\n", path_str(&path)));
    assert_eq!(
        crate::support::git(&path, &["rev-parse", "HEAD"]),
        origin.head()
    );
    fs::write(path.join("marker"), "kept").expect("marker");

    let again = run(&env, &["clone", "foo"]);
    assert!(again.status.success());
    assert_eq!(stdout(&again), format!("{}\n", path_str(&path)));
    assert!(path.join("marker").exists());
    assert_eq!(access_count(&env, SLUG), 2);
}

#[test]
fn edit_clones_then_runs_the_editor_on_the_path() {
    let (env, _origin) = env_with_foo();
    let path = env.repo_path(SLUG);
    let out = run(&env, &["edit", "foo"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert_eq!(env.editor.calls(), [vec![path_str(&path).to_owned()]]);
    assert!(path.join(".git").is_dir());
    assert_eq!(access_count(&env, SLUG), 1);
}

#[test]
fn a_failing_editor_is_an_error_and_records_nothing() {
    let (env, _origin) = env_with_foo();
    env.editor.set_exit(3);
    let out = run(&env, &["edit", "foo"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out).lines().count(), 1, "{}", stderr(&out));
    assert_eq!(access_count(&env, SLUG), 0);
}

#[test]
fn open_clones_and_opens_the_directory() {
    let (env, _origin) = env_with_foo();
    let path = env.repo_path(SLUG);
    let out = run(&env, &["open", "foo"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert_eq!(
        env.opener.wait_for_calls(1),
        [vec![path_str(&path).to_owned()]]
    );
    assert!(path.join(".git").is_dir());
    assert_eq!(access_count(&env, SLUG), 1);
}

#[test]
fn browse_opens_the_html_url_and_never_clones() {
    let (env, _origin) = env_with_foo();
    let out = run(&env, &["browse", "foo"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert_eq!(
        env.opener.wait_for_calls(1),
        [vec!["https://github.com/acme/foo".to_owned()]]
    );
    assert!(!env.repo_path(SLUG).exists());
    assert_eq!(access_count(&env, SLUG), 1);
}

#[test]
fn a_failed_clone_prints_nothing_and_records_nothing() {
    let (env, origin) = env_with_foo();
    fs::remove_dir_all(origin.bare_path()).expect("remove origin");
    for args in [&["sh-cd", "foo"][..], &["clone", "foo"], &["edit", "foo"]] {
        let out = run(&env, args);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        assert_eq!(stdout(&out), "");
        assert_eq!(stderr(&out).lines().count(), 1, "{}", stderr(&out));
    }
    assert_eq!(env.editor.calls(), Vec::<Vec<String>>::new());
    assert_eq!(access_count(&env, SLUG), 0);
}

fn ambiguous_env() -> TestEnv {
    let env = TestEnv::new();
    env.config()
        .domain(DomainCfg::new("github.com").default_domain())
        .write();
    env.seed_cache(&[
        RepoSeed::new("github.com/acme/foobar"),
        RepoSeed::new("github.com/acme/foobaz"),
    ]);
    env
}

#[test]
fn ambiguity_without_a_tty_is_a_list_error() {
    let env = ambiguous_env();
    for cmd in ["sh-cd", "sh-on", "edit", "clone", "browse", "open"] {
        let out = run(&env, &[cmd, "foob"]);
        assert_eq!(out.status.code(), Some(1), "{cmd}");
        assert_eq!(stdout(&out), "", "{cmd}");
        assert_eq!(
            stderr(&out),
            "work: ambiguous query 'foob' (use a more specific query, or run in a terminal \
             to pick)\n  - github.com/acme/foobar\n  - github.com/acme/foobaz\n",
            "{cmd}"
        );
    }
    assert_eq!(env.opener.calls(), Vec::<Vec<String>>::new());
    assert_eq!(cache_json(&env)["frecency"], serde_json::json!({}));
}

#[test]
fn pick_without_a_tty_is_an_error_when_several_match() {
    let env = ambiguous_env();
    let out = run(&env, &["sh-cd", "foob", "--pick"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stdout(&out), "");
    assert_eq!(
        stderr(&out),
        "work: --pick requires an interactive terminal\n"
    );
}

#[test]
fn pick_without_a_tty_resolves_a_single_match() {
    let env = ambiguous_env();
    let out = run(&env, &["browse", "foobaz", "--pick"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(
        env.opener.wait_for_calls(1),
        [vec!["https://github.com/acme/foobaz".to_owned()]]
    );
}

#[test]
fn dominant_query_history_auto_resolves_without_a_tty() {
    let env = TestEnv::new();
    env.config()
        .domain(DomainCfg::new("github.com").default_domain())
        .write();
    let now = jiff::Timestamp::now().to_string();
    env.cache()
        .repos(&[
            RepoSeed::new("github.com/acme/foobar"),
            RepoSeed::new("github.com/acme/foobaz"),
        ])
        .query_frecency("foob", "github.com/acme/foobaz", 3, &now)
        .write();
    let out = run(&env, &["browse", "foob"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(
        env.opener.wait_for_calls(1),
        [vec!["https://github.com/acme/foobaz".to_owned()]]
    );
    assert_eq!(
        cache_json(&env)["query_frecency"]["foob"]["github.com/acme/foobaz"]["access_count"],
        4
    );
}

#[test]
fn an_unknown_query_is_not_found() {
    let (env, _origin) = env_with_foo();
    let out = run(&env, &["sh-cd", "zzzz"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stdout(&out), "");
    assert_eq!(stderr(&out), "work: repository not found: zzzz\n");
}

/// A `work_dir` holding a newline, which no wrapper script can carry.
fn newline_work_dir_env() -> (TestEnv, PathBuf) {
    let env = TestEnv::new();
    let work_dir = env.home().join("nl\ndir");
    let toml_value = path_str(&work_dir).replace('\n', r"\n");
    env.write_config_raw(&format!("work_dir = \"{toml_value}\"\n"));
    let origin = env.origin("acme", "foo");
    env.seed_cache(&[RepoSeed::new(SLUG).origin(&origin)]);
    (env, work_dir)
}

#[test]
fn an_unprintable_path_fails_before_cloning_or_creating_anything() {
    let (env, work_dir) = newline_work_dir_env();
    for args in [&["sh-cd", "foo"][..], &["sh-cd"], &["sh-on", "foo"]] {
        let out = run(&env, args);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        assert_eq!(stdout(&out), "", "{args:?}");
        assert_eq!(stderr(&out).lines().count(), 1, "{}", stderr(&out));
        assert!(
            stderr(&out).contains("contains a newline"),
            "{}",
            stderr(&out)
        );
        assert!(!work_dir.exists(), "{args:?} created {work_dir:?}");
    }
    assert_eq!(access_count(&env, SLUG), 0);
}

#[test]
fn sh_on_rejects_an_editor_with_control_characters_before_cloning() {
    let (env, _origin) = env_with_foo();
    for editor in ["editor\necho INJECTED", "editor\recho", "ed\titor"] {
        let out = env
            .command()
            .args(["sh-on", "foo"])
            .env("WORK_EDITOR", editor)
            .output()
            .expect("run");
        assert_eq!(out.status.code(), Some(1), "{editor:?}");
        assert_eq!(stdout(&out), "", "{editor:?}");
        assert_eq!(stderr(&out).lines().count(), 1, "{}", stderr(&out));
        assert!(stderr(&out).contains("editor command"), "{}", stderr(&out));
    }
    assert!(!env.repo_path(SLUG).exists());
    assert_eq!(access_count(&env, SLUG), 0);
}
