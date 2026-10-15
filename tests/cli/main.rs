//! The single CLI integration-test binary: one module per command group.
#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "tests drive the binary and fixtures directly"
)]

mod about;
mod auth;
mod completions;
mod config;
mod frecency;
mod invariants;
mod list;
mod meta;
mod nav;
mod setup;
mod shell;
mod support;
mod sync;
mod update;
