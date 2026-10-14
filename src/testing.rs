//! Test doubles for the four seams, plus shared helpers (`#[cfg(test)]`).
//!
//! The fakes are contracts like any other type: their constructors and
//! inspection methods are fixed here, and the owning chunk fills in the
//! bodies (`git` W2.7, `gh` W2.6, `github` W2.8, `interact` W1.8). They are
//! consumed by the service chunks (W3.2, W3.3, W3.4, W4.1). Every fake is
//! `Sync` and records its calls behind a `Mutex`, which is fine in tests.

pub mod gh;
pub mod git;
pub mod github;
pub mod interact;

pub use gh::{FakeGh, GhCall};
pub use git::{FakeGit, GitCall, GitOp, RepoScript};
pub use github::{ApiCall, Endpoint, FakeGitHub, NullGitHub};
pub use interact::{Answer, PromptLog, ScriptedInteraction};

use std::io::{self, Write};
use std::sync::{Arc, Mutex, PoisonError};

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

    use super::SharedBuf;

    #[test]
    fn shared_buf_clones_see_the_same_bytes() {
        let reader = SharedBuf::new();
        let mut writer = reader.clone();
        writer.write_all(b"hello\n").expect("in-memory write");
        assert_eq!(reader.contents(), "hello\n");
    }
}
