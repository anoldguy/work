//! `about`: the non-TTY fallback and the unknown-style error.

use predicates::prelude::*;

use crate::support::TestEnv;

#[test]
fn without_a_tty_the_static_logo_goes_to_stderr() {
    let env = TestEnv::new();
    env.command()
        .arg("about")
        .assert()
        .success()
        .stdout("")
        .stderr(predicate::str::contains("██╗    ██╗ ██████╗"))
        .stderr(predicate::str::contains(format!(
            "v{}",
            env!("CARGO_PKG_VERSION")
        )))
        .stderr(predicate::str::contains("Made with love by anoldguy"))
        .stderr(predicate::str::contains("github.com/anoldguy/work"))
        .stderr(predicate::str::contains("\x1b").not());
}

#[test]
fn a_known_hidden_style_still_prints_the_static_logo_without_a_tty() {
    let env = TestEnv::new();
    env.command()
        .args(["about", "--style", "fire"])
        .assert()
        .success()
        .stdout("")
        .stderr(predicate::str::contains("github.com/anoldguy/work"));
}

#[test]
fn an_unknown_style_is_a_list_error_naming_every_style() {
    let env = TestEnv::new();
    let assert = env
        .command()
        .args(["about", "--style", "nope"])
        .assert()
        .code(1)
        .stdout("");
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");
    let mut lines = stderr.lines();
    let summary = lines.next().expect("summary line");
    assert!(
        summary.starts_with("work: unknown style \"nope\""),
        "{stderr}"
    );
    let items: Vec<&str> = lines.collect();
    assert_eq!(
        items,
        [
            "  - classic",
            "  - decode",
            "  - plasma",
            "  - fire",
            "  - dropin"
        ]
    );
}
