//! The pure core: search, resolution, frecency, paths, org filtering and sync
//! classification.
//!
//! Nothing under `core` performs I/O, reads the environment, or reads the
//! clock; "now" is always a parameter. It depends only on `domain`, std,
//! serde and nucleo-matcher. Note the module name shadows the `core` crate
//! inside this crate's root; refer to it as `crate::core`.

pub mod frecency;
pub mod org_filter;
pub mod paths;
pub mod query_frecency;
pub mod resolve;
pub mod search;
pub mod sync_outcome;
