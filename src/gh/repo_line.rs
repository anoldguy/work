//! One line of `gh api --jq '.[] | {name, owner: .owner.login, clone_url,
//! html_url}'` output, and its validation into a [`Repo`].
//!
//! Unknown fields are ignored. Validation is where an entry is rejected (and
//! counted as skipped by the caller).

use serde::Deserialize;

use crate::domain::{CloneUrl, Domain, HtmlUrl, NameError, Org, Repo, RepoName, Slug};

/// The `--jq` filter every repo listing uses: one object per line.
pub const REPO_JQ: &str = ".[] | {name, owner: .owner.login, clone_url, html_url}";

/// One line as gh prints it.
#[derive(Debug, Deserialize)]
struct RepoLine {
    name: String,
    owner: String,
    clone_url: String,
    html_url: String,
}

/// Parses and validates one output line into a [`Repo`] on `domain`. `None`
/// when the line is not the expected JSON or fails validation.
#[must_use]
pub fn parse_repo_line(line: &[u8], domain: &Domain) -> Option<Repo> {
    let line: RepoLine = serde_json::from_slice(line).ok()?;
    line.into_repo(domain).ok()
}

impl RepoLine {
    /// Validates the entry into a [`Repo`] on `domain`. Stricter than the
    /// domain types: API data must have an `https://` `clone_url` and
    /// `html_url`, both on `domain` (`file://`/`ssh://` clone URLs exist only
    /// for local fixtures, and an API response must not be able to point
    /// `git clone` at a local path).
    fn into_repo(self, domain: &Domain) -> Result<Repo, NameError> {
        let org = Org::try_from(self.owner)?;
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
    use super::parse_repo_line;
    use crate::domain::Domain;

    fn dom() -> Domain {
        "github.com".parse().unwrap()
    }

    fn line(name: &str, owner: &str, clone_url: &str, html_url: &str) -> Vec<u8> {
        serde_json::json!({
            "name": name, "owner": owner, "clone_url": clone_url, "html_url": html_url,
        })
        .to_string()
        .into_bytes()
    }

    fn urls(clone_url: &str, html_url: &str) -> Vec<u8> {
        line("widget", "Acme", clone_url, html_url)
    }

    #[test]
    fn valid_line_becomes_a_repo_on_the_domain() {
        let repo = parse_repo_line(
            &urls(
                "https://github.com/Acme/widget.git",
                "https://github.com/Acme/widget",
            ),
            &dom(),
        )
        .unwrap();
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
            let parsed = parse_repo_line(&urls(bad, "https://github.com/Acme/widget"), &dom());
            assert!(parsed.is_none(), "{bad:?}");
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
            ("https://github.com/Acme/widget.git", "javascript:alert(1)"),
        ];
        for (clone, html) in cases {
            assert!(
                parse_repo_line(&urls(clone, html), &dom()).is_none(),
                "{clone:?} {html:?}"
            );
        }
        let ok = urls(
            "https://GitHub.com/acme/WIDGET.git",
            "https://github.com/ACME/widget",
        );
        assert!(parse_repo_line(&ok, &dom()).is_some());
    }

    #[test]
    fn invalid_names_and_malformed_lines_are_rejected() {
        let url = |o: &str, n: &str| {
            (
                format!("https://github.com/{o}/{n}.git"),
                format!("https://github.com/{o}/{n}"),
            )
        };
        let (c, h) = url("acme", "..");
        assert!(parse_repo_line(&line("..", "acme", &c, &h), &dom()).is_none());
        let (c, h) = url("-x", "w");
        assert!(parse_repo_line(&line("w", "-x", &c, &h), &dom()).is_none());
        for junk in [&b""[..], b"not json", b"{\"name\":\"w\"}", b"[1,2]"] {
            assert!(parse_repo_line(junk, &dom()).is_none());
        }
    }
}
