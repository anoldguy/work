//! The repository cache.

pub mod fetch;
pub mod merge;
pub mod model;
pub mod refresh;
pub mod store;

pub use model::{CACHE_FILE, CACHE_VERSION, Cache, STALE_AFTER};

use std::io;
use std::path::PathBuf;

/// A cache failure. Paths render with `{:?}` to stay on one line.
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    /// No readable cache file, in a mode that does not refresh.
    #[error("no usable repository cache (run 'work update')")]
    Missing,
    /// Writing, renaming, or creating the cache directory failed.
    #[error("cannot write cache {path:?}: {source}")]
    Write {
        /// The file or directory.
        path: PathBuf,
        /// The I/O error.
        source: io::Error,
    },
    /// Serialising the cache failed.
    #[error("cannot encode the repository cache: {source}")]
    Encode {
        /// The `serde_json` error (single-line).
        source: serde_json::Error,
    },
    /// A refresh failed for every domain; nothing was written.
    #[error("could not fetch repositories from any domain (see the warnings above)")]
    AllFailed,
}
