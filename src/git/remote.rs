//! Remote URL inspection for `sync`'s token scoping (pure).

use crate::domain::Domain;

/// The host of a remote URL in any form git accepts (https, ssh, scp-like),
/// or `None` for local paths and unparsable input.
///
/// The host is lowercased; user info and port are dropped. IPv6 literals
/// and other hosts that are not a valid [`Domain`] give `None`.
#[must_use]
pub fn host_of(url: &str) -> Option<Domain> {
    let host = match url.split_once("://") {
        Some((scheme, after)) => {
            let scheme_ok = !scheme.is_empty()
                && scheme
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'));
            if !scheme_ok || scheme.eq_ignore_ascii_case("file") {
                return None;
            }
            let end = after.find(['/', '?', '#']).unwrap_or(after.len());
            let authority = &after[..end];
            let hostport = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
            match hostport.split_once(':') {
                Some((host, port)) if port.bytes().all(|b| b.is_ascii_digit()) => host,
                Some(_) => return None,
                None => hostport,
            }
        }
        None => scp_host(url)?,
    };
    host.parse().ok()
}

/// The host of a scp-like `[user@]host:path`. As in git, a `/` before the
/// first `:` makes it a local path.
fn scp_host(url: &str) -> Option<&str> {
    let (userhost, path) = url.split_once(':')?;
    if userhost.contains('/') || path.is_empty() {
        return None;
    }
    Some(userhost.rsplit_once('@').map_or(userhost, |(_, h)| h))
}

/// True for `https://` remotes, the only ones `sync` attaches a token to.
#[must_use]
pub fn is_https(url: &str) -> bool {
    url.get(..8)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use proptest::prelude::*;

    use super::*;

    fn host(url: &str) -> Option<String> {
        host_of(url).map(String::from)
    }

    #[test]
    fn https_ssh_and_scp_forms() {
        for url in [
            "https://github.com/acme/foo.git",
            "https://x-access-token@github.com/acme/foo",
            "https://GitHub.com:443/acme/foo",
            "https://github.com",
            "ssh://git@github.com/acme/foo.git",
            "ssh://git@github.com:22/acme/foo.git",
            "git@github.com:acme/foo.git",
            "github.com:acme/foo",
        ] {
            assert_eq!(host(url).as_deref(), Some("github.com"), "{url}");
        }
    }

    #[test]
    fn local_and_garbage_have_no_host() {
        for url in [
            "",
            "/srv/git/foo.git",
            "./foo",
            "../a:b",
            "file:///srv/git/foo.git",
            "https://",
            "https://[::1]/foo",
            "https://host:abc/foo",
            "git@:foo",
            "host:",
            "ht tp://github.com/x",
            // A scheme, not scp `host:path`: the authority is empty.
            "a://",
            "github.com:///acme/foo",
        ] {
            assert_matches!(host_of(url), None, "{url}");
        }
    }

    #[test]
    fn is_https_only_for_https_scheme() {
        assert!(is_https("https://github.com/a/b"));
        assert!(is_https("HTTPS://github.com/a/b"));
        assert!(!is_https("http://github.com/a/b"));
        assert!(!is_https("ssh://git@github.com/a/b"));
        assert!(!is_https("git@github.com:a/b"));
        assert!(!is_https("https:/"));
        assert!(!is_https(""));
    }

    proptest! {
        #[test]
        fn host_is_recovered_from_every_form(
            host in "[a-z][a-z0-9]{0,10}(\\.[a-z][a-z0-9]{0,10}){0,3}",
            user in proptest::option::of("[a-z][a-z0-9_-]{0,8}"),
            port in proptest::option::of(1u16..),
            path in "[A-Za-z0-9_./-]{1,20}",
            form in 0u8..3,
        ) {
            let user_at = user.map(|u| format!("{u}@")).unwrap_or_default();
            let colon_port = port.map(|p| format!(":{p}")).unwrap_or_default();
            // `host://…` is a URL with scheme `host` to git, not scp-like.
            prop_assume!(form != 2 || !path.starts_with("//"));
            let url = match form {
                0 => format!("https://{user_at}{host}{colon_port}/{path}"),
                1 => format!("ssh://{user_at}{host}{colon_port}/{path}"),
                _ => format!("{user_at}{host}:{path}"),
            };
            prop_assert_eq!(host_of(&url).map(String::from), Some(host), "{}", url);
        }

        #[test]
        fn never_panics(url in ".{0,60}") {
            let _ = host_of(&url);
            let _ = is_https(&url);
        }
    }
}
