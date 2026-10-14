//! Validated value types shared by every layer (pure).
//!
//! Each type here can only be built through a fallible constructor, so holding
//! one is proof that it is valid. Serde goes through the same constructors
//! (`#[serde(try_from = "String", into = "String")]`), so a cache or config
//! file cannot smuggle in an invalid value either.

pub mod name;
pub mod query;
pub mod repo;
pub mod slug;
pub mod time;
pub mod url;

pub use name::{Domain, Org, RepoName};
pub use query::QueryKey;
pub use repo::Repo;
pub use slug::Slug;
pub use time::StoredTime;
pub use url::{ApiUrl, CloneUrl, HtmlUrl};

/// Why a value failed validation.
///
/// `given` fields hold the rejected input and are rendered with `{:?}`, so an
/// input containing a newline or control character still yields a
/// single-line message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    /// Not a lowercase hostname (labels `[a-z0-9-]`, dot-separated; no
    /// scheme, port, or path).
    #[error("invalid domain {given:?} ({reason})")]
    Domain {
        /// The rejected input.
        given: String,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// Not a valid GitHub login or org name.
    #[error("invalid org {given:?} ({reason})")]
    Org {
        /// The rejected input.
        given: String,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// Not a valid repository name.
    #[error("invalid repository name {given:?} ({reason})")]
    RepoName {
        /// The rejected input.
        given: String,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// Not a `{domain}/{org}/{repo}` slug.
    #[error("invalid repository slug {given:?} ({reason})")]
    Slug {
        /// The rejected input.
        given: String,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// Not an allowed clone URL.
    #[error("invalid clone url {given:?} ({reason})")]
    CloneUrl {
        /// The rejected input.
        given: String,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// Not an allowed web URL.
    #[error("invalid web url {given:?} ({reason})")]
    HtmlUrl {
        /// The rejected input.
        given: String,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// Not an allowed API base URL.
    #[error("invalid api_url {given:?} ({reason})")]
    ApiUrl {
        /// The rejected input.
        given: String,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// Not a usable repo query (empty, control characters, or more than two
    /// slashes).
    #[error("invalid query {given:?} ({reason})")]
    Query {
        /// The rejected input.
        given: String,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// Not an RFC 3339 timestamp in the supported range.
    #[error("invalid timestamp {given:?} ({reason})")]
    Time {
        /// The rejected input.
        given: String,
        /// What is wrong with it.
        reason: &'static str,
    },
}
