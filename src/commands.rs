//! One module per command group.
//!
//! Every command returns `Result<ExitCode, Error>`, so commands such as
//! `auth status` and `sync` can print their results and still exit 1.
//! Stage-1 commands (`meta`, `completions`, `about`) take only the
//! environment and the `Ui`, so they work without a config; every other
//! command takes `&mut App`. Arguments are taken by value: `lib::run` owns
//! the parsed `Cli` and hands each command its part.

pub mod about;
pub mod auth;
pub mod completions;
pub mod config;
pub mod favorite;
pub mod list;
pub mod meta;
pub mod nav;
pub mod resolve;
pub mod setup;
pub mod sync;
pub mod update;
