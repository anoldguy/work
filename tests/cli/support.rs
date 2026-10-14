//! Shared helpers for the CLI tests. W1.14 owns this file and `support/*`
//! (`TestEnv`, fixtures, shims, the fake API); until then it only holds the
//! command builder.

use assert_cmd::Command;

/// A `work` command with an empty environment, so the developer's `HOME`,
/// `GITHUB_TOKEN` and `XDG_*` never leak into a test.
pub fn work() -> Command {
    let mut cmd: Command = assert_cmd::cargo_bin_cmd!("work");
    cmd.env_clear();
    cmd
}
