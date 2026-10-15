//! `work` with no arguments, `--version`, and usage errors.

use predicates::prelude::*;

use crate::support::{TestEnv, work};

#[test]
fn no_arguments_prints_help_to_stdout_and_exits_0() {
    work()
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stderr(predicate::str::is_empty());
}

#[test]
fn version_flag_prints_the_version() {
    work()
        .arg("--version")
        .assert()
        .success()
        .stdout(format!("work {}\n", env!("CARGO_PKG_VERSION")))
        .stderr(predicate::str::is_empty());
}

#[test]
fn help_flag_goes_to_stdout() {
    work()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Commands:"))
        .stderr(predicate::str::is_empty());
}

#[test]
fn a_bad_flag_gives_one_stderr_line_and_exit_2() {
    let assert = work().arg("--bogus").assert().code(2).stdout("");
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
    assert!(stderr.starts_with("work: unexpected argument"), "{stderr}");
    assert!(stderr.ends_with("(see 'work --help')\n"), "{stderr}");
}

#[test]
fn an_unknown_subcommand_is_a_one_line_usage_error() {
    let assert = work().arg("frobnicate").assert().code(2).stdout("");
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
}

#[test]
fn a_group_without_a_subcommand_is_a_one_line_usage_error() {
    let assert = work().arg("auth").assert().code(2).stdout("");
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");
    assert_eq!(
        stderr,
        "work: a subcommand is required (see 'work auth --help')\n"
    );
}

#[test]
fn a_newline_in_an_argument_stays_on_one_line() {
    let assert = work().arg("fr\nob").assert().code(2).stdout("");
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
    assert!(stderr.contains("fr\\nob"), "{stderr}");
}

#[test]
fn a_usage_error_survives_a_closed_stderr() {
    // stderr is a pipe nobody reads: writing the error must not panic (101).
    use std::process::{Command, Stdio};
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("work"))
        .arg("--bogus")
        .env_clear()
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    drop(child.stderr.take());
    assert_eq!(child.wait().expect("wait").code(), Some(2));
}

#[test]
fn version_subcommand_prints_the_package_version() {
    let env = TestEnv::new();
    env.command()
        .arg("version")
        .assert()
        .success()
        .stdout(format!("{}\n", env!("CARGO_PKG_VERSION")))
        .stderr(predicate::str::is_empty());
}

#[test]
fn activate_prints_the_bash_wrapper_byte_for_byte() {
    let env = TestEnv::new();
    env.command()
        .args(["activate", "bash"])
        .assert()
        .success()
        .stdout(include_str!("../../shell/work.bash"))
        .stderr(predicate::str::is_empty());
}

#[test]
fn activate_prints_the_zsh_and_fish_wrappers() {
    let env = TestEnv::new();
    env.command()
        .args(["activate", "zsh"])
        .assert()
        .success()
        .stdout(include_str!("../../shell/work.zsh"));
    env.command()
        .args(["activate", "fish"])
        .assert()
        .success()
        .stdout(include_str!("../../shell/work.fish"));
}

#[test]
fn activate_detects_the_shell_from_shell() {
    let env = TestEnv::new();
    env.command()
        .env("SHELL", "/usr/bin/fish")
        .arg("activate")
        .assert()
        .success()
        .stdout(include_str!("../../shell/work.fish"));
    env.command()
        .env("SHELL", "/bin/zsh")
        .arg("activate")
        .assert()
        .success()
        .stdout(include_str!("../../shell/work.zsh"));
}

#[test]
fn activate_falls_back_to_bash_when_shell_is_unset_or_unknown() {
    let env = TestEnv::new();
    env.command()
        .arg("activate")
        .assert()
        .success()
        .stdout(include_str!("../../shell/work.bash"));
    env.command()
        .env("SHELL", "/bin/tcsh")
        .arg("activate")
        .assert()
        .success()
        .stdout(include_str!("../../shell/work.bash"));
}

#[test]
fn activate_with_an_unknown_shell_is_a_one_line_usage_error() {
    let env = TestEnv::new();
    let assert = env
        .command()
        .args(["activate", "tcsh"])
        .assert()
        .code(2)
        .stdout("");
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
    assert!(stderr.starts_with("work: invalid value 'tcsh'"), "{stderr}");
}
