//! The pure core: search, resolution, frecency, paths, org filtering and sync
//! classification. The name shadows the `core` crate; refer to it as
//! `crate::core`.

pub mod frecency;
pub mod org_filter;
pub mod paths;
pub mod query_frecency;
pub mod resolve;
pub mod search;
pub mod sync_outcome;
