//! [`Repo`]: one cached repository (pure).

use serde::{Deserialize, Serialize};

use super::{CloneUrl, HtmlUrl, Slug};

/// One repository as stored in the cache.
///
/// The JSON shape is `{"slug": "...", "clone_url": "...", "html_url": "..."}`;
/// the domain is part of the slug.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repo {
    /// `{domain}/{org}/{repo}`.
    pub slug: Slug,
    /// What `git clone` is given.
    pub clone_url: CloneUrl,
    /// The repository's web page.
    pub html_url: HtmlUrl,
}
