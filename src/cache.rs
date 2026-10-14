//! The repository cache: model, merge, file store, concurrent fetch, and the
//! freshness policy that ties them together.

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
    /// No cache file, in a mode that does not refresh (`pull`).
    #[error("no repository cache yet (run 'work update')")]
    Missing,
    /// The cache file or its directory cannot be read.
    #[error("cannot read cache {path:?}: {source}")]
    Read {
        /// The file or directory.
        path: PathBuf,
        /// The I/O error.
        source: io::Error,
    },
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
    /// A refresh failed for every domain; nothing was written. The per-domain
    /// causes were already printed as warnings.
    #[error("could not fetch repositories from any domain (see the warnings above)")]
    AllFailed,
}
