//! Shared harness for the CLI tests (W1.14).
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
//! - [`Shim`] (`shims.rs`): recording `gh`, editor and opener executables.
//! - [`FakeApi`] (`api.rs`): a loopback HTTP/1.1 server with a routes table,
//!   `Link` paging, status codes and request recording.
//!
//! Rules: no network beyond loopback, no sleeps, no wall-clock assertions.

mod api;
mod env;
mod git;
mod shims;

#[allow(unused_imports, reason = "each test file uses a different subset")]
pub use api::{FakeApi, HostApi, RecordedRequest, Response};
#[allow(unused_imports, reason = "each test file uses a different subset")]
pub use env::{CacheSeed, ConfigSeed, DomainCfg, RepoSeed, TestEnv};
#[allow(unused_imports, reason = "each test file uses a different subset")]
pub use git::{Origin, git};
#[allow(unused_imports, reason = "each test file uses a different subset")]
pub use shims::Shim;

use assert_cmd::Command;

/// A `work` command with an empty environment, so the developer's `HOME`,
/// `GITHUB_TOKEN` and `XDG_*` never leak into a test. Prefer
/// [`TestEnv::command`], which also sets up a home, `PATH` and shims.
pub fn work() -> Command {
    let mut cmd: Command = assert_cmd::cargo_bin_cmd!("work");
    cmd.env_clear();
    cmd
}
