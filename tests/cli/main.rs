//! Integration tests: the built binary, real `git`, and a fake `gh`.
#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "tests drive the binary and fixtures directly"
)]

mod ambiguity;
mod clone;
mod commands;
mod config;
mod frecency;
mod shell;
mod support;
mod sync;
mod update;
