//! `work completions`.

use crate::support::TestEnv;

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn prints_script_for_each_shell() {
    let env = TestEnv::new();
    for (shell, marker) in [
        ("bash", "complete -F _work_with_repos"),
        ("zsh", "#compdef work"),
        ("fish", "complete -c work"),
    ] {
        let out = env.command().args(["completions", shell]).output().unwrap();
        assert!(out.status.success(), "{shell}");
        let text = stdout(&out);
        assert!(text.contains(marker), "{shell}");
        assert!(text.contains("work list --complete"), "{shell}");
    }
}

#[test]
fn detects_shell_from_env() {
    let env = TestEnv::new();
    let out = env
        .command()
        .env("SHELL", "/usr/bin/fish")
        .arg("completions")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(stdout(&out).contains("complete -c work"));
}

#[test]
fn install_writes_fish_file_and_reports_path() {
    let env = TestEnv::new();
    let out = env
        .command()
        .args(["completions", "fish", "--install"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(stdout(&out), "");
    let path = env.home().join(".config/fish/completions/work.fish");
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("complete -c work"));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains(&path.display().to_string()), "{err}");
}

#[test]
fn install_zsh_creates_dirs_and_hints_fpath() {
    let env = TestEnv::new();
    let out = env
        .command()
        .args(["completions", "zsh", "--install"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let path = env.home().join(".local/share/zsh/site-functions/_work");
    assert!(path.is_file());
    assert!(String::from_utf8_lossy(&out.stderr).contains("fpath"));
}

#[test]
#[cfg(target_os = "macos")]
fn install_bash_uses_homebrew_prefix_when_set() {
    let env = TestEnv::new();
    let prefix = env.home().join("brew");
    let out = env
        .command()
        .env("HOMEBREW_PREFIX", &prefix)
        .args(["completions", "bash", "--install"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(prefix.join("etc/bash_completion.d/work").is_file());
}

#[test]
fn unknown_shell_is_a_usage_error() {
    let env = TestEnv::new();
    let out = env
        .command()
        .args(["completions", "tcsh"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stdout(&out), "");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.starts_with("work: "), "{err}");
    assert_eq!(err.trim_end().lines().count(), 1, "{err}");
}

#[test]
#[cfg(not(target_os = "macos"))]
fn install_ignores_homebrew_prefix_off_macos() {
    let env = TestEnv::new();
    let prefix = env.home().join("brew");
    let out = env
        .command()
        .env("HOMEBREW_PREFIX", &prefix)
        .args(["completions", "bash", "--install"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(!prefix.exists());
    let path = env
        .home()
        .join(".local/share/bash-completion/completions/work");
    assert!(path.is_file());
}

#[test]
fn hidden_items_are_not_completed() {
    let env = TestEnv::new();
    for shell in ["bash", "zsh", "fish"] {
        let out = env.command().args(["completions", shell]).output().unwrap();
        let text = stdout(&out);
        for hidden in ["sh-cd", "sh-on", "--style", "commands --sh"] {
            assert!(!text.contains(hidden), "{shell} leaks {hidden}");
        }
        // The only mention of `--complete` is the hook's own call.
        assert_eq!(
            text.matches("--complete").count(),
            text.matches("list --complete").count(),
            "{shell}"
        );
    }
}

/// Runs `body` in `bin` with a fake `work` on PATH that lists two repos;
/// `None` when the shell is not installed.
fn run_shell(env: &TestEnv, bin: &str, args: &[&str], body: &str) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let dir = env.home().join("shell-test");
    std::fs::create_dir_all(&dir).unwrap();
    let fake = dir.join("work");
    std::fs::write(&fake, "#!/bin/sh\nprintf 'acme/alpha\\nacme/beta\\n'\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = std::process::Command::new(bin)
        .args(args)
        .arg(body)
        .env_clear()
        .env("HOME", env.home())
        .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
        .output()
        .ok()?;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn script_file(env: &TestEnv, shell: &str) -> std::path::PathBuf {
    let out = env.command().args(["completions", shell]).output().unwrap();
    let path = env.home().join(format!("work-completions.{shell}"));
    std::fs::write(&path, out.stdout).unwrap();
    path
}

#[test]
fn bash_completes_visible_subcommands_and_repos() {
    const BASH: [&str; 2] = ["--norc", "--noprofile"];
    let env = TestEnv::new();
    let file = script_file(&env, "bash");
    let reply = |words: &str, cword: usize| {
        format!(
            "source {}; COMP_WORDS=({words}); COMP_CWORD={cword}; COMP_LINE=\"{words}\"; \
             COMP_POINT=${{#COMP_LINE}}; _work_with_repos work; printf '%s\\n' \"${{COMPREPLY[@]}}\"",
            file.display()
        )
    };
    let run = |words: &str, cword: usize| {
        run_shell(
            &env,
            "bash",
            &[BASH[0], BASH[1], "-c"],
            &reply(words, cword),
        )
    };
    let Some(subs) = run("work ''", 1) else {
        return;
    };
    assert!(subs.lines().any(|l| l == "cd"), "{subs}");
    assert!(!subs.contains("sh-cd"), "{subs}");
    let flags = run("work list --", 2).unwrap();
    assert!(!flags.contains("--complete"), "{flags}");
    let repos = run("work cd --pick acme/al", 3).unwrap();
    assert_eq!(repos.trim(), "acme/alpha");
}

#[test]
fn fish_completes_visible_subcommands_and_repos() {
    let env = TestEnv::new();
    let file = script_file(&env, "fish");
    let run = |line: &str| {
        run_shell(
            &env,
            "fish",
            &["--no-config", "-c"],
            &format!("source {}; complete -C '{line}'", file.display()),
        )
    };
    let Some(subs) = run("work ") else {
        return;
    };
    assert!(subs.lines().any(|l| l.starts_with("cd\t")), "{subs}");
    assert!(!subs.contains("sh-cd"), "{subs}");
    let flags = run("work list --").unwrap();
    assert!(!flags.contains("--complete"), "{flags}");
    let repos = run("work cd acme/al").unwrap();
    assert!(repos.contains("acme/alpha"), "{repos}");
    let shells = run("work completions ").unwrap();
    assert!(shells.contains("zsh"), "{shells}");
}
