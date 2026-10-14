//! The embedded wrapper scripts printed by `work activate`.

use crate::shell::Shell;

/// The wrapper for `shell`, byte-for-byte from `shell/work.<shell>`
/// (embedded with `include_str!`).
#[must_use]
pub fn activation_script(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash => include_str!("../../shell/work.bash"),
        Shell::Zsh => include_str!("../../shell/work.zsh"),
        Shell::Fish => include_str!("../../shell/work.fish"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::quote;
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Stdio};

    #[test]
    fn bash_snapshot() {
        insta::assert_snapshot!(activation_script(Shell::Bash));
    }

    #[test]
    fn zsh_snapshot() {
        insta::assert_snapshot!(activation_script(Shell::Zsh));
    }

    #[test]
    fn fish_snapshot() {
        insta::assert_snapshot!(activation_script(Shell::Fish));
    }

    /// Sources the real wrapper in `shell` with a fake `work` binary on PATH
    /// that prints a `sh-cd` style script, then checks the eval changed
    /// directory and other commands pass through. Skips when the shell is
    /// absent.
    fn wrapper_evals(shell: Shell) {
        let bin = shell.name();
        let probe = Command::new(bin)
            .args(["-c", "true"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if probe.is_err() {
            eprintln!("note: {bin} not on PATH; skipping wrapper eval test");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let bindir = dir.path().join("bin");
        std::fs::create_dir(&bindir).unwrap();
        let target = dir.path().join("it's a \\ dir");
        std::fs::create_dir(&target).unwrap();
        let target = std::fs::canonicalize(&target).unwrap();

        let script_file = dir.path().join("script.txt");
        std::fs::write(
            &script_file,
            format!("cd {}\n", quote(target.to_str().unwrap())),
        )
        .unwrap();
        let fake = bindir.join("work");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\ncase \"$1\" in\n  commands) echo cd;;\n  sh-cd) cat {};;\n  *) echo passthrough \"$@\";;\nesac\n",
                quote(script_file.to_str().unwrap())
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

        let wrapper = dir.path().join(format!("work.{shell}"));
        std::fs::write(&wrapper, activation_script(shell)).unwrap();
        let w = quote(wrapper.to_str().unwrap());
        let src = match shell {
            Shell::Fish => format!("source {w}; work cd x; pwd; work other a"),
            _ => format!(". {w}; work cd x; pwd; work other a"),
        };
        let path = format!(
            "{}:{}",
            bindir.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut cmd = Command::new(bin);
        match shell {
            Shell::Fish => cmd.args(["--no-config", "-c", &src]),
            Shell::Zsh => cmd.args(["-f", "-c", &src]),
            Shell::Bash => cmd.args(["--norc", "--noprofile", "-c", &src]),
        };
        let out = cmd
            .env_clear()
            .env("PATH", path)
            .env("HOME", dir.path())
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let expected = format!("{}\npassthrough other a\n", target.display());
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            expected,
            "{bin}: stderr {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn bash_wrapper_evals_a_cd_script() {
        wrapper_evals(Shell::Bash);
    }

    #[test]
    fn zsh_wrapper_evals_a_cd_script() {
        wrapper_evals(Shell::Zsh);
    }

    #[test]
    fn fish_wrapper_evals_a_cd_script() {
        wrapper_evals(Shell::Fish);
    }
}
