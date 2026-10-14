//! `config`: summary, `path`, `--edit` and `default`.

use predicates::prelude::*;

use crate::support::{DomainCfg, TestEnv};

fn stdout_of(env: &TestEnv, args: &[&str]) -> String {
    let assert = env.command().args(args).assert().success();
    String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8")
}

#[test]
fn summary_lists_the_effective_configuration() {
    let env = TestEnv::new();
    env.config()
        .domain(
            DomainCfg::new("github.com")
                .default_domain()
                .favorites(&["my-team", "my-company"])
                .no_api_url(),
        )
        .domain(
            DomainCfg::new("ghe.example.com")
                .extras(&["guests"])
                .no_api_url(),
        )
        .exclude_orgs(&["archived-org"])
        .write();
    let out = stdout_of(&env, &["config"]);
    let home = env.home().display().to_string();
    insta::assert_snapshot!(out.replace(&home, "<HOME>"));
}

#[test]
fn summary_without_domains_names_the_implicit_one() {
    let env = TestEnv::new();
    env.write_config_raw("");
    let out = stdout_of(&env, &["config"]);
    assert!(out.contains("implicit github.com"), "{out}");
}

#[test]
fn path_reports_a_missing_file() {
    let env = TestEnv::new();
    let out = stdout_of(&env, &["config", "path"]);
    assert_eq!(out, format!("{}\n", env.config_path().display()));
}

#[test]
fn path_reports_an_existing_file_even_when_it_is_invalid() {
    let env = TestEnv::new();
    env.write_config_raw("this is = = not toml");
    let out = stdout_of(&env, &["config", "path"]);
    assert_eq!(out, format!("{}\n", env.config_path().display()));
}

#[test]
fn edit_opens_the_config_in_the_editor_even_when_it_is_invalid() {
    let env = TestEnv::new();
    env.write_config_raw("this is = = not toml");
    env.command().args(["config", "--edit"]).assert().success();
    let calls = env.editor.wait_for_calls(1);
    assert_eq!(calls, [[env.config_path().display().to_string()]]);
}

#[test]
fn edit_creates_the_config_directory() {
    let env = TestEnv::new();
    env.command().args(["config", "--edit"]).assert().success();
    assert!(env.config_path().parent().expect("parent").is_dir());
    assert_eq!(env.editor.wait_for_calls(1).len(), 1);
}

#[test]
fn edit_reports_a_failing_editor() {
    let env = TestEnv::new();
    env.editor.set_exit(3);
    env.command()
        .args(["config", "--edit"])
        .assert()
        .failure()
        .stdout("")
        .stderr(predicate::str::starts_with("work: "));
}

#[test]
fn default_moves_the_flag_and_keeps_comments() {
    let env = TestEnv::new();
    env.write_config_raw(
        "# my settings\n[domains.\"github.com\"]\nis_default = true # main\n\n[domains.\"ghe.example.com\"]\n# corp\nfavorite_orgs = [\"corp\"]\n",
    );
    env.command()
        .args(["config", "default", "ghe.example.com"])
        .assert()
        .success()
        .stdout("");
    let text = std::fs::read_to_string(env.config_path()).expect("read");
    assert!(text.contains("# my settings"), "{text}");
    assert!(text.contains("# main"), "{text}");
    assert!(text.contains("# corp"), "{text}");
    assert!(text.contains("is_default = false"), "{text}");
    assert!(text.contains("is_default = true"), "{text}");
    let after = text.split("ghe.example.com").nth(1).expect("section");
    assert!(after.contains("is_default = true"), "{text}");
}

#[test]
fn default_rejects_an_unknown_domain_with_a_hint() {
    let env = TestEnv::new();
    env.config()
        .domain(DomainCfg::new("github.com").default_domain().no_api_url())
        .write();
    let assert = env
        .command()
        .args(["config", "default", "nope.example.com"])
        .assert()
        .code(1)
        .stdout("");
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(
        stderr.contains("not configured (run 'work setup"),
        "{stderr}"
    );
}

#[test]
fn default_without_a_domain_needs_a_terminal() {
    let env = TestEnv::new();
    env.config()
        .domain(DomainCfg::new("github.com").default_domain().no_api_url())
        .write();
    env.command()
        .args(["config", "default"])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::contains("requires an interactive terminal"));
}

#[test]
fn default_without_a_domain_and_no_domains_says_so() {
    let env = TestEnv::new();
    env.write_config_raw("");
    env.command()
        .args(["config", "default"])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::contains("no domains are configured"));
}
