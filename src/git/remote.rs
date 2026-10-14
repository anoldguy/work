//! Remote URL inspection for `sync`'s token scoping (pure).

use crate::domain::Domain;

/// The host of a remote URL in any form git accepts (https, ssh, scp-like),
/// or `None` for local paths and unparsable input.
#[must_use]
pub fn host_of(url: &str) -> Option<Domain> {
    let _ = url;
    unimplemented!("W2.7")
}

/// True for `https://` remotes, the only ones `sync` attaches a token to.
#[must_use]
pub fn is_https(url: &str) -> bool {
    let _ = url;
    unimplemented!("W2.7")
}
