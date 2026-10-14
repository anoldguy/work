//! GitHub REST API: the seam trait, the HTTP client, API base selection,
//! Link paging, and wire DTOs.

pub mod base;
pub mod client;
pub mod dto;
pub mod link;

pub use base::api_base;
pub use client::HttpGitHub;
pub use link::{Link, parse_link_header};

use std::fmt::{self, Debug};

use crate::auth::SecretToken;
use crate::domain::{Domain, Org, Repo};

/// `per_page` on every list request.
pub const PER_PAGE: u32 = 100;

/// Pages followed per listing before stopping with `Listing::truncated`.
pub const MAX_PAGES: u32 = 100;

/// Per-page progress for one listing, passed to the `on_page` callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    /// 1-based page number just received.
    pub number: u32,
    /// Valid repos received so far, this page included.
    pub repos_so_far: u32,
}

/// The result of one repo listing.
#[derive(Debug, Default)]
pub struct Listing {
    /// Valid repos, in API order.
    pub repos: Vec<Repo>,
    /// Entries dropped because they failed validation (for example an
    /// `html_url` on another host).
    pub skipped: u32,
    /// True when [`MAX_PAGES`] was reached and more pages remained.
    pub truncated: bool,
}

/// What `work` asks of the GitHub API. `Sync` because fetch workers share
/// one `&dyn GitHubApi` across scoped threads.
pub trait GitHubApi: Sync + Debug {
    /// `GET /user`: the authenticated login.
    ///
    /// # Errors
    ///
    /// Any [`ApiError`].
    fn viewer(&self, d: &Domain, t: &SecretToken) -> Result<Org, ApiError>;

    /// `GET /user/orgs`, all pages.
    ///
    /// # Errors
    ///
    /// Any [`ApiError`].
    fn member_orgs(&self, d: &Domain, t: &SecretToken) -> Result<Vec<Org>, ApiError>;

    /// `GET /user/repos?affiliation=owner,collaborator`, all pages.
    ///
    /// # Errors
    ///
    /// Any [`ApiError`].
    fn user_repos(
        &self,
        d: &Domain,
        t: &SecretToken,
        on_page: &mut dyn FnMut(Page),
    ) -> Result<Listing, ApiError>;

    /// `GET /orgs/{org}/repos?type=all`, all pages.
    ///
    /// # Errors
    ///
    /// Any [`ApiError`].
    fn org_repos(
        &self,
        d: &Domain,
        t: &SecretToken,
        org: &Org,
        on_page: &mut dyn FnMut(Page),
    ) -> Result<Listing, ApiError>;
}

/// A failed API call. `path` is the request path without query string;
/// headers and tokens never appear.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// A non-success HTTP status.
    #[error("HTTP {status} from {path}{hint}")]
    Status {
        /// The status code.
        status: u16,
        /// Request path, e.g. `/orgs/acme/repos`.
        path: String,
        /// An actionable hint for 401 and rate-limited 403.
        hint: StatusHint,
    },
    /// The request could not be completed (DNS, TLS, connection, timeout).
    #[error("request to {path} failed: {detail}")]
    Transport {
        /// Request path.
        path: String,
        /// The transport error, one line.
        detail: String,
    },
    /// The body was not the expected JSON.
    #[error("unexpected response from {path}: {detail}")]
    Decode {
        /// Request path.
        path: String,
        /// The decode error, one line.
        detail: String,
    },
    /// A `Link: rel="next"` pointed at a different scheme, host or port than
    /// the API base; it was not followed, so the token was not sent there.
    #[error("refusing to follow a pagination link to another origin ({origin:?})")]
    Pagination {
        /// The foreign `scheme://host:port` (server-supplied, so rendered
        /// with `{:?}`).
        origin: String,
    },
}

/// The parenthesised hint on [`ApiError::Status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusHint {
    /// No hint.
    None,
    /// 401: the token is missing or expired.
    Login {
        /// The host to log in to.
        domain: Domain,
    },
    /// 403 with `x-ratelimit-remaining: 0`.
    RateLimit,
}

impl fmt::Display for StatusHint {
    /// Empty, or a leading space and a parenthesised hint.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => Ok(()),
            Self::Login { domain } => write!(f, " (run 'gh auth login --hostname {domain}')"),
            Self::RateLimit => f.write_str(" (API rate limit exceeded; try again later)"),
        }
    }
}
