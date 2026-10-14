//! Shared harness for the CLI tests.
//!
//! Every `tests/cli/<group>.rs` file builds on [`TestEnv`]:
//!
//! ```ignore
//! let env = TestEnv::new();
//! env.config().domain(DomainCfg::new("github.com").default_domain()).write();
//! env.api.host("github.com").paged("/user/repos", &[page1, page2]);
//! env.command().arg("update").assert().success();
//! assert_eq!(env.api.host("github.com").requests().len(), 3);
//! ```
//!
//! Pieces:
//! - [`TestEnv`] (`env.rs`): tempdir `HOME` and `XDG_*`, a fully controlled
//!   command (`env_clear`, `PATH` = shim dir only), config and cache seeding.
//! - [`Origin`] (`git.rs`): a bare `file://` origin built with real `git`.
//! - [`shims::Shim`] (`shims.rs`): recording `gh`, editor and opener executables.
//! - [`FakeApi`] (`api.rs`): a loopback HTTP/1.1 server with a routes table,
//!   `Link` paging, status codes and request recording.
//!
//! Rules: no network beyond loopback, no sleeps, no wall-clock assertions.

mod api;
mod env;
mod git;
mod shims;

pub use api::{FakeApi, Response};
pub use env::{DomainCfg, RepoSeed, TestEnv};
pub use git::{Origin, git};

use assert_cmd::Command;

/// A `work` command with an empty environment, so the developer's `HOME`,
/// `GITHUB_TOKEN` and `XDG_*` never leak into a test. Prefer
/// [`TestEnv::command`], which also sets up a home, `PATH` and shims.
pub fn work() -> Command {
    let mut cmd: Command = assert_cmd::cargo_bin_cmd!("work");
    cmd.env_clear();
    cmd
}
