//! `setup`.

use predicates::prelude::*;

use crate::support::TestEnv;

#[test]
fn without_a_terminal_it_is_a_one_line_error() {
    let env = TestEnv::new();
    env.command()
        .arg("setup")
        .assert()
        .code(1)
        .stdout("")
        .stderr("work: setup requires an interactive terminal\n");
    assert!(!env.config_path().exists());
}

#[test]
fn a_named_domain_without_a_terminal_is_the_same_error() {
    let env = TestEnv::new();
    env.command()
        .args(["setup", "ghe.example.com"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("requires an interactive terminal"));
}
