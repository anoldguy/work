//! Wire shapes of the GitHub REST responses `work` reads, and their
//! validation into domain types.
//!
//! Unknown fields are ignored. Conversion is where an entry that fails
//! validation is rejected (and counted as skipped by the client).

use serde::Deserialize;

use crate::domain::{CloneUrl, Domain, HtmlUrl, NameError, Org, Repo, RepoName, Slug};

/// `GET /user` and the `owner` of a repo.
#[derive(Debug, Deserialize)]
pub struct UserDto {
    /// The login.
    pub login: String,
}

/// One entry of `GET /user/orgs`.
#[derive(Debug, Deserialize)]
pub struct OrgDto {
    /// The org login.
    pub login: String,
}

/// One entry of a repo listing.
#[derive(Debug, Deserialize)]
pub struct RepoDto {
    /// Repository name.
    pub name: String,
    /// Owner (user or org).
    pub owner: UserDto,
    /// HTTPS clone URL.
    pub clone_url: String,
    /// Web page.
    pub html_url: String,
}

impl RepoDto {
    /// Validates the entry into a [`Repo`] on `domain`. Stricter than the
    /// domain types: API data must have an `https://` `clone_url` and
    /// `html_url`, both on `domain` (`file://`/`ssh://` clone URLs exist only
    /// for local fixtures, and an API response must not be able to point
    /// `git clone` at a local path).
    ///
    /// # Errors
    ///
    /// A [`NameError`] when any part is invalid, or either URL is not https on
    /// `domain` (no explicit port) at `/{owner}/{name}` (`.git` for the
    /// clone URL).
    pub fn into_repo(self, domain: &Domain) -> Result<Repo, NameError> {
        let org = Org::try_from(self.owner.login)?;
        let name = RepoName::try_from(self.name)?;
        let repo_path = format!("/{org}/{name}");
        let clone_url = api_clone_url(self.clone_url, domain, &repo_path)?;
        let html_url = HtmlUrl::try_from(self.html_url)?;
        if let Some(reason) = placement_problem(html_url.as_str(), domain, &repo_path, "") {
            return Err(NameError::HtmlUrl {
                given: html_url.into(),
                reason,
            });
        }
        Ok(Repo {
            slug: Slug::new(domain.clone(), org, name),
            clone_url,
            html_url,
        })
    }
}

/// Why an https URL (already a valid [`HtmlUrl`]) is not exactly
/// `https://{domain}{repo_path}{suffix}`: host and path compare ignoring
/// ASCII case, and an explicit port is refused.
fn placement_problem(
    url: &str,
    domain: &Domain,
    repo_path: &str,
    suffix: &str,
) -> Option<&'static str> {
    let rest = url.strip_prefix("https://")?;
    let end = rest.find('/').unwrap_or(rest.len());
    let (authority, path) = rest.split_at(end);
    if authority.contains(':') {
        return Some("an explicit port is not allowed");
    }
    if !authority.eq_ignore_ascii_case(domain.as_str()) {
        return Some("host is not the repository's domain");
    }
    let path_ok = path
        .strip_suffix(suffix)
        .is_some_and(|p| p.eq_ignore_ascii_case(repo_path));
    (!path_ok).then_some("path does not match the repository's owner and name")
}

/// A clone URL from the API: it must pass both the [`HtmlUrl`] rules (https,
/// a host, no user info) and the [`CloneUrl`] rules, and be exactly
/// `https://{domain}{repo_path}.git`.
fn api_clone_url(raw: String, domain: &Domain, repo_path: &str) -> Result<CloneUrl, NameError> {
    let https = HtmlUrl::try_from(raw).map_err(|e| match e {
        NameError::HtmlUrl { given, reason } => NameError::CloneUrl { given, reason },
        other => other,
    })?;
    if let Some(reason) = placement_problem(https.as_str(), domain, repo_path, ".git") {
        return Err(NameError::CloneUrl {
            given: https.into(),
            reason,
        });
    }
    CloneUrl::try_from(String::from(https))
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::{RepoDto, UserDto};
    use crate::domain::{Domain, NameError};

    fn dom() -> Domain {
        "github.com".parse().unwrap()
    }

    fn dto(clone_url: &str, html_url: &str) -> RepoDto {
        RepoDto {
            name: "widget".to_owned(),
            owner: UserDto {
                login: "Acme".to_owned(),
            },
            clone_url: clone_url.to_owned(),
            html_url: html_url.to_owned(),
        }
    }

    fn good() -> RepoDto {
        dto(
            "https://github.com/Acme/widget.git",
            "https://github.com/Acme/widget",
        )
    }

    #[test]
    fn valid_entry_becomes_a_repo_on_the_domain() {
        let repo = good().into_repo(&dom()).unwrap();
        assert_eq!(repo.slug.to_string(), "github.com/Acme/widget");
        assert_eq!(
            repo.clone_url.as_str(),
            "https://github.com/Acme/widget.git"
        );
        assert_eq!(repo.html_url.as_str(), "https://github.com/Acme/widget");
    }

    #[test]
    fn clone_url_must_be_https_on_the_domain() {
        for bad in [
            "file:///tmp/evil.git",
            "ssh://git@github.com/Acme/widget.git",
            "git@github.com:Acme/widget.git",
            "http://github.com/Acme/widget.git",
            "https://evil.com/Acme/widget.git",
            "https://github.com.evil.com/x.git",
            "https://tok@github.com/x.git",
            "-oProxyCommand=x",
        ] {
            assert_matches!(
                dto(bad, "https://github.com/Acme/widget").into_repo(&dom()),
                Err(NameError::CloneUrl { .. }),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn urls_must_point_at_this_repo_without_a_port() {
        let cases = [
            (
                "https://github.com:444/Acme/widget.git",
                "https://github.com/Acme/widget",
            ),
            (
                "https://github.com/evil/payload.git",
                "https://github.com/Acme/widget",
            ),
            (
                "https://github.com/Acme/widget",
                "https://github.com/Acme/widget",
            ),
            (
                "https://github.com/Acme/widget.git",
                "https://github.com:444/Acme/widget",
            ),
            (
                "https://github.com/Acme/widget.git",
                "https://github.com/evil/payload",
            ),
            (
                "https://github.com/Acme/widget.git",
                "https://github.com/Acme/widget/x",
            ),
        ];
        for (clone, html) in cases {
            assert_matches!(
                dto(clone, html).into_repo(&dom()),
                Err(NameError::CloneUrl { .. } | NameError::HtmlUrl { .. }),
                "{clone:?} {html:?}"
            );
        }
        let ok = dto(
            "https://GitHub.com/acme/WIDGET.git",
            "https://github.com/ACME/widget",
        );
        assert!(ok.into_repo(&dom()).is_ok());
    }

    #[test]
    fn html_url_must_be_https_on_the_domain() {
        for bad in [
            "http://github.com/Acme/widget",
            "https://evil.com/Acme/widget",
            "javascript:alert(1)",
        ] {
            assert_matches!(
                dto("https://github.com/Acme/widget.git", bad).into_repo(&dom()),
                Err(NameError::HtmlUrl { .. }),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn invalid_names_are_rejected() {
        let mut bad = good();
        bad.name = "../x".to_owned();
        assert_matches!(bad.into_repo(&dom()), Err(NameError::RepoName { .. }));
        let mut bad = good();
        bad.owner.login = "-x".to_owned();
        assert_matches!(bad.into_repo(&dom()), Err(NameError::Org { .. }));
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let json = r#"{"name":"w","owner":{"login":"a","id":1},"clone_url":"https://github.com/a/w.git","html_url":"https://github.com/a/w","private":true}"#;
        let dto: RepoDto = serde_json::from_str(json).unwrap();
        assert!(dto.into_repo(&dom()).is_ok());
    }
}
