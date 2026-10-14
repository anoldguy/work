//! API base selection (pure).

use crate::domain::{ApiUrl, Domain};

/// The API base for `d`: the configured `api_url` when present, else
/// `https://api.github.com` for `github.com`, `https://api.<host>` for
/// `*.ghe.com`, and `https://<host>/api/v3` for any other (GHES) host.
#[must_use]
pub fn api_base(d: &Domain, configured: Option<&ApiUrl>) -> ApiUrl {
    let _ = (d, configured);
    unimplemented!("W1.3")
}
