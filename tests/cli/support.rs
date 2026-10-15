//! Shared harness for the CLI tests.
//!
//! Every `tests/cli/<group>.rs` file builds on [`TestEnv`]:
//!
//! ```ignore
//! let env = TestEnv::new();
//! env.config().domain(DomainCfg::new("github.com").default_domain()).write();
//! env.gh.set_api("github.com", "user", &["me".to_owned()]);
//! env.gh.set_api("github.com", "user/repos", &[repo_line("github.com", "me", "x")]);
//! env.command().arg("update").assert().success();
//! ```
//!
//! Pieces:
//! - [`TestEnv`] (`env.rs`): tempdir `HOME` and `XDG_*`, a fully controlled
//!   command (`env_clear`, `PATH` = shim dir only), config and cache seeding.
//! - [`Origin`] (`git.rs`): a bare `file://` origin built with real `git`.
//! - [`shims::Shim`] (`shims.rs`): recording `gh`, editor and opener
//!   executables; the `gh` shim answers `gh api` from per-host fixtures.
//!
//! Rules: no network, no sleeps, no wall-clock assertions.

mod env;
mod git;
mod shims;

pub use env::{DomainCfg, RepoSeed, TestEnv};
pub use git::{Origin, git};

use assert_cmd::Command;

/// One line of `gh api --jq` repo-listing output for `host/owner/name`, as
/// the `gh` shim fixtures hold it.
#[must_use]
pub fn repo_line(host: &str, owner: &str, name: &str) -> String {
    serde_json::json!({
        "name": name,
        "owner": owner,
        "clone_url": format!("https://{host}/{owner}/{name}.git"),
        "html_url": format!("https://{host}/{owner}/{name}"),
    })
    .to_string()
}

/// A `work` command with an empty environment, so the developer's `HOME`,
/// `GITHUB_TOKEN` and `XDG_*` never leak into a test. Prefer
/// [`TestEnv::command`], which also sets up a home, `PATH` and shims.
pub fn work() -> Command {
    let mut cmd: Command = assert_cmd::cargo_bin_cmd!("work");
    cmd.env_clear();
    cmd
}
