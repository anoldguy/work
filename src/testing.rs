//! Test doubles for the four seams, plus shared helpers (`#[cfg(test)]`).
//!
//! Each fake is scripted through its constructor and builder methods and
//! reports what it was asked through inspection methods. Every fake is
//! `Sync` and records its calls behind a `Mutex`, which is fine in tests.

pub mod gh;
pub mod git;
pub mod interact;

pub use gh::{ApiCall, Endpoint, FakeGh, GhCall};
pub use git::{FakeGit, GitCall, GitOp, RepoScript};
pub use interact::{Answer, PromptLog, ScriptedInteraction};

use std::io::{self, ErrorKind, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex, PoisonError};
use std::{fs, thread};

/// The guard every [`install_script`] body starts with: a probe run exits
/// at once.
const PROBE_GUARD: &str = "if [ -n \"$WORK_SHIM_PROBE\" ]; then exit 0; fi";

/// Writes `#!/bin/sh` plus `body` to `path` as an executable that is safe
/// to exec immediately, even under `cargo test`'s threaded runner.
///
/// The script is written to a private temporary name, closed, renamed into
/// place, and then probed (run with `WORK_SHIM_PROBE` set, which makes it
/// exit at once) until the probe stops failing with `ETXTBSY`. A sibling
/// test thread that forked while our write handle was open keeps a copy of
/// it until its child execs; exec'ing the script during that window fails
/// with "Text file busy". The probe yields between attempts and never
/// sleeps. Mirrors `install_script` in `tests/cli/support/shims.rs`.
///
/// # Panics
///
/// Panics when the script cannot be written or run (tests only).
pub fn install_script(path: &Path, body: &str) {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.tmp", std::process::id()));
    let tmp = Path::new(&tmp);
    fs::write(tmp, format!("#!/bin/sh\n{PROBE_GUARD}\n{body}\n")).expect("write script");
    fs::set_permissions(tmp, fs::Permissions::from_mode(0o755)).expect("chmod script");
    fs::rename(tmp, path).expect("rename script into place");
    loop {
        match Command::new(path)
            .env_clear()
            .env("WORK_SHIM_PROBE", "1")
            .output()
        {
            Ok(_) => return,
            Err(err) if err.kind() == ErrorKind::ExecutableFileBusy => thread::yield_now(),
            Err(err) => panic!("probe {}: {err}", path.display()),
        }
    }
}

/// An in-memory writer whose clones share one buffer: hand one clone to
/// `Ui::for_test` and read the output back from another.
#[derive(Debug, Clone, Default)]
pub struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl SharedBuf {
    /// An empty buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything written so far, lossily decoded.
    #[must_use]
    pub fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::{SharedBuf, install_script};

    #[test]
    fn shared_buf_clones_see_the_same_bytes() {
        let reader = SharedBuf::new();
        let mut writer = reader.clone();
        writer.write_all(b"hello\n").expect("in-memory write");
        assert_eq!(reader.contents(), "hello\n");
    }

    #[test]
    fn installed_scripts_run_at_once_and_ignore_the_probe() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("hello");
        install_script(&path, "echo hi; exit 3");
        let out = std::process::Command::new(&path)
            .env_clear()
            .output()
            .expect("run script");
        assert_eq!(out.status.code(), Some(3));
        assert_eq!(out.stdout, b"hi\n");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read dir")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(leftovers, [std::ffi::OsString::from("hello")]);
    }
}
