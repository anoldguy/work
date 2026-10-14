//! [`HttpGitHub`]: the blocking ureq client.

use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::sync::OnceLock;
use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use ureq::config::RedirectAuthHeaders;
use ureq::http::Response;
use ureq::tls::{RootCerts, TlsConfig};
use ureq::{Body, Proxy};

use crate::auth::{SecretToken, redact};
use crate::domain::{ApiUrl, Domain, Org};
use crate::github::dto::{OrgDto, RepoDto, UserDto};
use crate::github::{
    ApiError, GitHubApi, Listing, MAX_PAGES, PER_PAGE, Page, StatusHint, api_base,
    parse_link_header,
};

/// The REST API version every request pins.
const API_VERSION: &str = "2022-11-28";

/// The media type every request accepts.
const ACCEPT: &str = "application/vnd.github+json";

/// `User-Agent` on every request.
const USER_AGENT: &str = concat!("work/", env!("CARGO_PKG_VERSION"));

/// Upper bound on one request, so a stalled server cannot hang `update`.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Longest foreign origin echoed in a [`ApiError::Pagination`].
const MAX_ORIGIN_LEN: usize = 80;

/// Same-origin redirects followed per request.
const MAX_REDIRECTS: u32 = 5;

/// Statuses whose `Location` is followed (all safe for a `GET`).
const REDIRECTS: [u16; 5] = [301, 302, 303, 307, 308];

/// Bytes of a 403 body read to look for a rate-limit message.
const MAX_ERROR_BODY: u64 = 4096;

/// The real API client.
///
/// Sends `Accept: application/vnd.github+json`,
/// `X-GitHub-Api-Version: 2022-11-28` and `User-Agent: work/<version>`; never
/// forwards auth headers across redirects; follows `rel="next"` only within
/// the API base's origin, for at most `MAX_PAGES` pages.
#[derive(Debug)]
pub struct HttpGitHub {
    /// Built on first request (by the private `agent()`), so constructing
    /// the client costs nothing for commands that never call the API.
    agent: OnceLock<ureq::Agent>,
    bases: BTreeMap<Domain, ApiUrl>,
}

impl HttpGitHub {
    /// A client using `bases` for configured domains and the computed
    /// default (`api_base`) for any other domain.
    #[must_use]
    pub const fn new(bases: BTreeMap<Domain, ApiUrl>) -> Self {
        Self {
            agent: OnceLock::new(),
            bases,
        }
    }

    /// The shared agent, built on first use.
    ///
    /// Status codes come back as responses (not errors) so they can be
    /// mapped to [`ApiError::Status`]; ureq follows no redirects (`get`
    /// follows same-origin ones itself, so a renamed org keeps its auth and
    /// a foreign one never sees the token); `HTTPS_PROXY`/`ALL_PROXY`/
    /// `NO_PROXY` are read explicitly; certificates are checked by the
    /// platform verifier so corporate CAs on GHES work.
    fn agent(&self) -> &ureq::Agent {
        self.agent.get_or_init(|| {
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .max_redirects(0)
                .redirect_auth_headers(RedirectAuthHeaders::Never)
                .user_agent(USER_AGENT)
                .accept(ACCEPT)
                .proxy(Proxy::try_from_env())
                .tls_config(
                    TlsConfig::builder()
                        .root_certs(RootCerts::PlatformVerifier)
                        .build(),
                )
                .timeout_global(Some(REQUEST_TIMEOUT))
                .build()
                .new_agent()
        })
    }

    /// The API base for `d`: configured, else computed.
    fn base(&self, d: &Domain) -> ApiUrl {
        api_base(d, self.bases.get(d))
    }

    /// One authenticated `GET`, following up to [`MAX_REDIRECTS`] redirects
    /// that stay on `base`'s origin (with auth). A redirect anywhere else is
    /// [`ApiError::Redirect`] and is not followed. `path` is the endpoint
    /// path used in errors (never the URL, which may carry a query). Returns
    /// the body and the `rel="next"` target, if any.
    fn get(
        &self,
        base: &ApiUrl,
        url: &str,
        path: &str,
        d: &Domain,
        t: &SecretToken,
    ) -> Result<(String, Option<String>), ApiError> {
        let transport = |e: &ureq::Error| ApiError::Transport {
            path: path.to_owned(),
            detail: one_line(&redact(&e.to_string(), &[t])),
        };
        let mut url = url.to_owned();
        let mut hops: u32 = 0;
        loop {
            let mut response = self
                .agent()
                .get(&url)
                .header("Authorization", format!("Bearer {}", t.expose()))
                .header("Accept", ACCEPT)
                .header("X-GitHub-Api-Version", API_VERSION)
                .call()
                .map_err(|e| transport(&e))?;
            let status = response.status().as_u16();
            if (200..300).contains(&status) {
                let next = next_link(&response);
                let body = response
                    .body_mut()
                    .read_to_string()
                    .map_err(|e| transport(&e))?;
                return Ok((body, next));
            }
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok());
            match location {
                Some(location) if REDIRECTS.contains(&status) && hops < MAX_REDIRECTS => {
                    let target = resolve_location(&url, location);
                    if !base.same_origin(&target) {
                        return Err(ApiError::Redirect {
                            path: path.to_owned(),
                            origin: origin_of(&target),
                        });
                    }
                    hops = hops.saturating_add(1);
                    url = target;
                }
                _ => return Err(status_error(status, path, d, &mut response)),
            }
        }
    }

    /// `GET` one JSON object.
    fn get_one<T: DeserializeOwned>(
        &self,
        d: &Domain,
        t: &SecretToken,
        path: &str,
    ) -> Result<T, ApiError> {
        let base = self.base(d);
        let url = format!("{base}{path}");
        let (body, _) = self.get(&base, &url, path, d, t)?;
        serde_json::from_str(&body).map_err(|e| decode(path, &e))
    }

    /// Follows `rel="next"` from `path?{query}per_page=100`, handing each
    /// page's JSON array to `each` with its 1-based number. Returns true
    /// when [`MAX_PAGES`] was reached with more pages remaining.
    fn paginate(
        &self,
        d: &Domain,
        t: &SecretToken,
        path: &str,
        query: &str,
        each: &mut dyn FnMut(u32, Vec<serde_json::Value>),
    ) -> Result<bool, ApiError> {
        let base = self.base(d);
        let mut url = format!("{base}{path}?{query}per_page={PER_PAGE}");
        let mut visited = HashSet::new();
        let mut number = 1;
        loop {
            let (body, next) = self.get(&base, &url, path, d, t)?;
            let entries: Vec<serde_json::Value> =
                serde_json::from_str(&body).map_err(|e| decode(path, &e))?;
            each(number, entries);
            let Some(next) = next else {
                return Ok(false);
            };
            if number >= MAX_PAGES {
                return Ok(true);
            }
            if !base.same_origin(&next) {
                return Err(ApiError::Pagination {
                    origin: origin_of(&next),
                });
            }
            visited.insert(std::mem::take(&mut url));
            if visited.contains(&next) {
                return Err(ApiError::PaginationLoop {
                    path: path.to_owned(),
                });
            }
            number = number.saturating_add(1);
            url = next;
        }
    }

    /// A repo listing at `path`, validated entry by entry.
    fn repos(
        &self,
        d: &Domain,
        t: &SecretToken,
        path: &str,
        query: &str,
        on_page: &mut dyn FnMut(Page),
    ) -> Result<Listing, ApiError> {
        let mut listing = Listing::default();
        let mut so_far: u32 = 0;
        listing.truncated = self.paginate(d, t, path, query, &mut |number, entries| {
            for entry in entries {
                match serde_json::from_value::<RepoDto>(entry)
                    .ok()
                    .and_then(|dto| dto.into_repo(d).ok())
                {
                    Some(repo) => {
                        listing.repos.push(repo);
                        so_far = so_far.saturating_add(1);
                    }
                    None => listing.skipped = listing.skipped.saturating_add(1),
                }
            }
            on_page(Page {
                number,
                repos_so_far: so_far,
            });
        })?;
        Ok(listing)
    }
}

impl GitHubApi for HttpGitHub {
    fn viewer(&self, d: &Domain, t: &SecretToken) -> Result<Org, ApiError> {
        const PATH: &str = "/user";
        let user: UserDto = self.get_one(d, t, PATH)?;
        Org::try_from(user.login).map_err(|e| decode(PATH, &e))
    }

    fn member_orgs(&self, d: &Domain, t: &SecretToken) -> Result<Vec<Org>, ApiError> {
        let mut orgs = Vec::new();
        // Truncation is ignored: 10,000 orgs is not a real membership list.
        self.paginate(d, t, "/user/orgs", "", &mut |_, entries| {
            orgs.extend(entries.into_iter().filter_map(|entry| {
                let dto = serde_json::from_value::<OrgDto>(entry).ok()?;
                Org::try_from(dto.login).ok()
            }));
        })?;
        Ok(orgs)
    }

    fn user_repos(
        &self,
        d: &Domain,
        t: &SecretToken,
        on_page: &mut dyn FnMut(Page),
    ) -> Result<Listing, ApiError> {
        self.repos(
            d,
            t,
            "/user/repos",
            "affiliation=owner,collaborator&",
            on_page,
        )
    }

    fn org_repos(
        &self,
        d: &Domain,
        t: &SecretToken,
        org: &Org,
        on_page: &mut dyn FnMut(Page),
    ) -> Result<Listing, ApiError> {
        // `Org` is validated (alphanumerics and `-`), so it is path-safe.
        let path = format!("/orgs/{org}/repos");
        self.repos(d, t, &path, "type=all&", on_page)
    }
}

/// Maps a non-2xx status to [`ApiError::Status`] with its hint. For a 403
/// it reads at most [`MAX_ERROR_BODY`] bytes of the body to spot GitHub's
/// secondary rate limit (the message itself is never shown).
fn status_error(status: u16, path: &str, d: &Domain, response: &mut Response<Body>) -> ApiError {
    let hint = match status {
        401 => StatusHint::Login { domain: d.clone() },
        429 => StatusHint::RateLimit,
        403 => forbidden_hint(d, response),
        404 if path.starts_with("/orgs/") => StatusHint::OrgNotFound,
        500..=599 => StatusHint::ServerError,
        _ => StatusHint::None,
    };
    ApiError::Status {
        status,
        path: path.to_owned(),
        hint,
    }
}

/// Why a 403 happened: rate limit, SAML SSO, or plain lack of access.
fn forbidden_hint(d: &Domain, response: &mut Response<Body>) -> StatusHint {
    let headers = response.headers();
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    if header("x-ratelimit-remaining").is_some_and(|v| v.trim() == "0")
        || header("retry-after").is_some()
    {
        return StatusHint::RateLimit;
    }
    let sso = header("x-github-sso").is_some();
    if body_mentions_rate_limit(response) {
        StatusHint::RateLimit
    } else if sso {
        StatusHint::Sso { domain: d.clone() }
    } else {
        StatusHint::Forbidden
    }
}

/// True when the error body's `message` mentions a rate limit.
fn body_mentions_rate_limit(response: &mut Response<Body>) -> bool {
    #[derive(Deserialize)]
    struct Message {
        message: String,
    }
    let mut raw = Vec::new();
    let read = response
        .body_mut()
        .as_reader()
        .take(MAX_ERROR_BODY)
        .read_to_end(&mut raw);
    read.is_ok()
        && serde_json::from_slice::<Message>(&raw)
            .is_ok_and(|m| m.message.to_ascii_lowercase().contains("rate limit"))
}

/// Resolves a `Location` value against the URL that returned it.
fn resolve_location(current: &str, location: &str) -> String {
    let scheme_end = location.find("://");
    let is_absolute = scheme_end.is_some_and(|i| {
        i > 0
            && location[..i]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    });
    if is_absolute {
        return location.to_owned();
    }
    let (scheme, _) = current.split_once("://").unwrap_or(("https", ""));
    if let Some(rest) = location.strip_prefix("//") {
        return format!("{scheme}://{rest}");
    }
    if location.starts_with('/') {
        return format!("{}{location}", origin_part(current));
    }
    // Relative path: replace the last segment of the current path.
    let without_query = current.split(['?', '#']).next().unwrap_or(current);
    let dir_end = without_query
        .rfind('/')
        .map_or(without_query.len(), |i| i + 1);
    let origin_len = origin_part(current).len();
    if dir_end <= origin_len {
        format!("{}/{location}", origin_part(current))
    } else {
        format!("{}{location}", &without_query[..dir_end])
    }
}

/// The `rel="next"` target across every `Link` header on `response`.
fn next_link(response: &Response<Body>) -> Option<String> {
    let joined = response
        .headers()
        .get_all("link")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect::<Vec<_>>()
        .join(", ");
    parse_link_header(&joined).next
}

/// A one-line [`ApiError::Decode`].
fn decode(path: &str, e: &impl std::fmt::Display) -> ApiError {
    ApiError::Decode {
        path: path.to_owned(),
        detail: one_line(&e.to_string()),
    }
}

/// Collapses line breaks and control characters into single spaces.
fn one_line(s: &str) -> String {
    s.split(char::is_control)
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The `scheme://authority` part of a server-supplied URL, without user
/// info, capped at [`MAX_ORIGIN_LEN`] characters.
fn origin_of(url: &str) -> String {
    let origin = if url.contains("://") {
        let raw = origin_part(url);
        let (scheme, authority) = raw.split_once("://").unwrap_or(("", raw));
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        format!("{scheme}://{host}")
    } else {
        url.to_owned()
    };
    origin.chars().take(MAX_ORIGIN_LEN).collect()
}

/// The `scheme://authority` prefix of `url` as written (user info kept),
/// or `url` itself when it has no `://`.
fn origin_part(url: &str) -> &str {
    url.split_once("://").map_or(url, |(scheme, rest)| {
        let end = rest.find(['/', '\\', '?', '#']).unwrap_or(rest.len());
        &url[..scheme.len() + 3 + end]
    })
}

#[cfg(test)]
mod tests {
    use super::{one_line, origin_of, resolve_location};

    #[test]
    fn resolve_location_handles_absolute_rooted_and_relative_forms() {
        let cur = "https://api.github.com/orgs/old/repos?page=2";
        assert_eq!(
            resolve_location(cur, "https://evil.com/x"),
            "https://evil.com/x"
        );
        assert_eq!(resolve_location(cur, "//evil.com/x"), "https://evil.com/x");
        assert_eq!(
            resolve_location(cur, "/organizations/1/repos?page=2"),
            "https://api.github.com/organizations/1/repos?page=2"
        );
        assert_eq!(
            resolve_location(cur, "other"),
            "https://api.github.com/orgs/old/other"
        );
        assert_eq!(
            resolve_location("http://127.0.0.1:9", "x"),
            "http://127.0.0.1:9/x"
        );
        assert_eq!(
            resolve_location(cur, "/x?u=http://evil.com"),
            "https://api.github.com/x?u=http://evil.com"
        );
    }

    #[test]
    fn origin_of_keeps_scheme_and_authority_without_user_info() {
        assert_eq!(origin_of("https://evil.com/x?y"), "https://evil.com");
        assert_eq!(
            origin_of("http://tok@evil.com:8080/x"),
            "http://evil.com:8080"
        );
        assert_eq!(origin_of("/relative?page=2"), "/relative?page=2");
        assert_eq!(origin_of(&"a".repeat(200)).len(), 80);
    }

    #[test]
    fn one_line_collapses_newlines() {
        assert_eq!(one_line("a\nb\r\n c\t"), "a b c");
        assert_eq!(one_line(""), "");
    }
}
