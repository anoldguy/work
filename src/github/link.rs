//! RFC 8288 `Link` header parsing for pagination (pure).

/// The relations `work` uses from a `Link` header.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Link {
    /// Target of `rel="next"`.
    pub next: Option<String>,
    /// Target of `rel="last"`.
    pub last: Option<String>,
}

/// Parses a `Link` header value. Never fails: malformed entries are skipped,
/// and the first occurrence of each relation wins.
#[must_use]
pub fn parse_link_header(value: &str) -> Link {
    let _ = value;
    unimplemented!("W1.3")
}
