//! API base selection (pure).

use crate::domain::{ApiUrl, Domain};

/// The API base for `d`: the configured `api_url` when present, else
/// `https://api.github.com` for `github.com`, `https://api.<host>` for
/// `*.ghe.com`, and `https://<host>/api/v3` for any other (GHES) host.
///
/// # Panics
///
/// Never in practice: a validated [`Domain`] always yields a valid https base.
#[must_use]
pub fn api_base(d: &Domain, configured: Option<&ApiUrl>) -> ApiUrl {
    if let Some(url) = configured {
        return url.clone();
    }
    let host = d.as_str();
    let url = if host == "github.com" {
        "https://api.github.com".to_owned()
    } else if host.ends_with(".ghe.com") {
        format!("https://api.{host}")
    } else {
        format!("https://{host}/api/v3")
    };
    // A validated `Domain` is a bare hostname, so these are valid https bases.
    ApiUrl::try_from(url).expect("https URL built from a validated Domain is a valid ApiUrl")
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use proptest::prelude::*;

    use super::api_base;
    use crate::domain::{ApiUrl, Domain};

    fn dom(s: &str) -> Domain {
        s.parse().unwrap()
    }

    #[test]
    fn github_com_default() {
        assert_eq!(
            api_base(&dom("github.com"), None).as_str(),
            "https://api.github.com"
        );
    }

    #[test]
    fn ghe_cloud_default() {
        assert_eq!(
            api_base(&dom("acme.ghe.com"), None).as_str(),
            "https://api.acme.ghe.com"
        );
    }

    #[test]
    fn ghes_default() {
        assert_eq!(
            api_base(&dom("git.corp.example"), None).as_str(),
            "https://git.corp.example/api/v3"
        );
    }

    #[test]
    fn configured_wins() {
        let cfg = ApiUrl::try_from("https://proxy.example/gh".to_owned()).unwrap();
        for d in ["github.com", "acme.ghe.com", "git.corp.example"] {
            let got = api_base(&dom(d), Some(&cfg));
            assert_matches!(got.as_str(), "https://proxy.example/gh");
        }
    }

    proptest! {
        #[test]
        fn default_base_is_always_a_valid_https_url(
            host in "[a-z0-9]{1,10}(\\.[a-z0-9-]{1,10}){0,3}(\\.ghe\\.com)?",
        ) {
            // Any host that is a valid Domain must not hit the `expect`.
            if let Ok(d) = host.parse::<Domain>() {
                prop_assert!(api_base(&d, None).as_str().starts_with("https://"));
            }
        }
    }
}
