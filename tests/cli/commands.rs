//! One smoke test per spec'd command behaviour nothing else covers.

use std::fs;

use crate::support::{TestEnv, is_running, text, wait_for};

#[test]
fn list_shows_the_cache_and_filters_by_substring() {
    let env = TestEnv::new();
    env.config_domains(&["github.com"]);
    env.seed_cache(&["github.com/acme/app", "github.com/acme/tool"], false);

    let out = env.command().arg("list").output().expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        "github.com/acme/app\ngithub.com/acme/tool\n"
    );
    let out = env
        .command()
        .args(["list", "--filter", "too"])
        .output()
        .expect("run");
    assert_eq!(text(&out.stdout), "github.com/acme/tool\n");
}

#[test]
fn browse_opens_the_web_url_without_cloning() {
    let env = TestEnv::new();
    env.config_domains(&["github.com"]);
    env.seed_cache(&["github.com/acme/app"], false);

    let out = env.command().args(["browse", "app"]).output().expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    let calls = wait_for("the opener", || {
        Some(env.calls("opener")).filter(|calls| !calls.is_empty())
    });
    assert!(
        calls[0].ends_with("\thttps://github.com/acme/app"),
        "{calls:?}"
    );
    assert!(!env.work_dir().join("acme").exists());
}

#[test]
fn a_detached_opener_exits_after_its_test_is_gone() {
    let env = TestEnv::new();
    env.config_domains(&["github.com"]);
    env.seed_cache(&["github.com/acme/app"], false);

    let out = env.command().args(["browse", "app"]).output().expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    let pid = wait_for("the opener's pid", || env.pids("opener").first().copied());
    drop(env);
    wait_for("the opener to exit", || (!is_running(pid)).then_some(()));
}

#[test]
fn auth_status_checks_every_domain_and_fails_if_one_is_logged_out() {
    let env = TestEnv::new();
    env.config_domains(&["github.com", "ghe.example.com"]);
    env.gh_auth_exit("ghe.example.com", 1);

    let out = env
        .command()
        .args(["auth", "status"])
        .output()
        .expect("run");
    assert!(!out.status.success());
    let calls = env.calls("gh");
    for host in ["github.com", "ghe.example.com"] {
        assert!(
            calls.contains(&format!("auth status --hostname {host}")),
            "{calls:?}"
        );
    }
}

#[test]
fn completions_print_and_install_without_hidden_commands() {
    let env = TestEnv::new();
    let out = env
        .command()
        .args(["completions", "bash"])
        .output()
        .expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    let script = text(&out.stdout);
    assert!(script.contains("browse"), "{script}");
    assert!(!script.contains("sh-cd"), "{script}");

    let out = env
        .command()
        .args(["completions", "bash", "--install"])
        .output()
        .expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    let installed = env
        .home()
        .join(".local/share/bash-completion/completions/work");
    assert_eq!(fs::read_to_string(installed).expect("installed"), script);
}

#[test]
fn bare_work_prints_help_without_hidden_commands() {
    let env = TestEnv::new();
    let out = env.command().output().expect("run");
    let help = format!("{}{}", text(&out.stdout), text(&out.stderr));
    assert!(help.contains("browse"), "{help}");
    assert!(!help.contains("sh-cd"), "{help}");
}

#[test]
fn cd_without_the_wrapper_points_to_activate() {
    let env = TestEnv::new();
    let out = env.command().args(["cd", "app"]).output().expect("run");
    assert!(!out.status.success());
    assert_eq!(text(&out.stdout), "");
    assert!(text(&out.stderr).contains("work activate"));
}

#[test]
fn about_without_a_terminal_prints_the_static_logo() {
    let env = TestEnv::new();
    let out = env.command().arg("about").output().expect("run");
    assert!(out.status.success());
    assert_eq!(text(&out.stdout), "");
    let stderr = text(&out.stderr);
    assert!(
        stderr.contains("╚███╔███╔╝╚██████╔╝██║  ██║██║  ██╗"),
        "{stderr}"
    );
    assert!(
        stderr.contains(&format!("v{}", env!("CARGO_PKG_VERSION"))),
        "{stderr}"
    );
    assert!(stderr.contains("github.com/anoldguy/work"), "{stderr}");

    let out = env
        .command()
        .args(["about", "--style", "nope"])
        .output()
        .expect("run");
    assert!(!out.status.success());
    let stderr = text(&out.stderr);
    for style in ["classic", "decode", "plasma", "fire", "dropin"] {
        assert!(stderr.contains(style), "{stderr}");
    }
}
