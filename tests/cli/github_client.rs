//! Tests for the GitHub HTTP client, run against `support::FakeApi`.

use std::assert_matches;
use std::collections::BTreeMap;

use work::auth::SecretToken;
use work::domain::{ApiUrl, Domain, Org};
use work::github::{ApiError, GitHubApi, HttpGitHub, MAX_PAGES, Page, StatusHint};

use crate::support::{FakeApi, Response};

const TOKEN: &str = "ghp_fakeTOKEN1234567890";

fn dom() -> Domain {
    "github.com".parse().unwrap()
}

fn token() -> SecretToken {
    SecretToken::new(TOKEN).unwrap()
}

fn client(api: &FakeApi) -> HttpGitHub {
    let base = ApiUrl::try_from(api.base_url()).unwrap();
    HttpGitHub::new(BTreeMap::from([(dom(), base)]))
}

/// One repo entry as the API returns it.
fn repo_json(owner: &str, name: &str) -> String {
    format!(
        r#"{{"name":"{name}","owner":{{"login":"{owner}"}},"clone_url":"https://github.com/{owner}/{name}.git","html_url":"https://github.com/{owner}/{name}","private":false}}"#
    )
}

fn page(entries: &[String]) -> String {
    format!("[{}]", entries.join(","))
}

#[test]
fn three_page_listing_calls_on_page_once_per_page() {
    let api = FakeApi::start();
    let pages = [
        page(&[repo_json("acme", "a"), repo_json("acme", "b")]),
        page(&[repo_json("acme", "c")]),
        page(&[repo_json("acme", "d")]),
    ];
    let pages: Vec<&str> = pages.iter().map(String::as_str).collect();
    api.paged("/orgs/acme/repos", &pages);

    let mut seen = Vec::new();
    let org: Org = "acme".parse().unwrap();
    let listing = client(&api)
        .org_repos(&dom(), &token(), &org, &mut |p| seen.push(p))
        .unwrap();

    assert_eq!(
        seen,
        [(1, 2), (2, 3), (3, 4)].map(|(number, repos_so_far)| Page {
            number,
            repos_so_far
        })
    );
    let slugs: Vec<String> = listing.repos.iter().map(|r| r.slug.to_string()).collect();
    assert_eq!(
        slugs,
        [
            "github.com/acme/a",
            "github.com/acme/b",
            "github.com/acme/c",
            "github.com/acme/d"
        ]
    );
    assert_eq!(listing.skipped, 0);
    assert!(!listing.truncated);

    let requests = api.distinct_requests_to("/orgs/acme/repos");
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].query_param("type").as_deref(), Some("all"));
    assert_eq!(requests[0].query_param("per_page").as_deref(), Some("100"));
    for request in &requests {
        let auth = format!("Bearer {TOKEN}");
        assert_eq!(request.authorization(), Some(auth.as_str()));
    }
}

#[test]
fn requests_send_the_github_headers() {
    let api = FakeApi::start();
    api.route("/user", Response::json(r#"{"login":"octocat"}"#));
    let login = client(&api).viewer(&dom(), &token()).unwrap();
    assert_eq!(login.as_str(), "octocat");

    let request = &api.requests_to("/user")[0];
    assert_eq!(
        request.header("accept"),
        Some("application/vnd.github+json")
    );
    assert_eq!(request.header("x-github-api-version"), Some("2022-11-28"));
    let ua = format!("work/{}", env!("CARGO_PKG_VERSION"));
    assert_eq!(request.header("user-agent"), Some(ua.as_str()));
}

#[test]
fn user_repos_asks_for_owner_and_collaborator_affiliation() {
    let api = FakeApi::start();
    api.route(
        "/user/repos",
        Response::json(&page(&[repo_json("me", "dots")])),
    );
    let listing = client(&api)
        .user_repos(&dom(), &token(), &mut |_| {})
        .unwrap();
    assert_eq!(listing.repos.len(), 1);
    let request = &api.requests_to("/user/repos")[0];
    assert_eq!(
        request.query_param("affiliation").as_deref(),
        Some("owner,collaborator")
    );
}

#[test]
fn member_orgs_follows_pages_and_skips_invalid_logins() {
    let api = FakeApi::start();
    api.paged(
        "/user/orgs",
        &[
            r#"[{"login":"acme"},{"login":"-bad"}]"#,
            r#"[{"login":"widgets"}]"#,
        ],
    );
    let orgs = client(&api).member_orgs(&dom(), &token()).unwrap();
    let names: Vec<&str> = orgs.iter().map(Org::as_str).collect();
    assert_eq!(names, ["acme", "widgets"]);
}

#[test]
fn off_origin_next_link_is_a_pagination_error_and_gets_no_token() {
    let api = FakeApi::start();
    let other = FakeApi::start();
    other.route("/steal", Response::json("[]"));
    api.route(
        "/user/repos",
        Response::json(&page(&[repo_json("me", "a")]))
            .link_next(&format!("{}/steal?page=2", other.base_url())),
    );
    let err = client(&api)
        .user_repos(&dom(), &token(), &mut |_| {})
        .unwrap_err();
    let expected_origin = other.base_url();
    assert_matches!(&err, ApiError::Pagination { origin } if *origin == expected_origin);
    assert_eq!(other.requests().len(), 0);
    assert!(!err.to_string().contains(TOKEN));
}

#[test]
fn cross_origin_redirect_is_refused_and_the_target_is_never_contacted() {
    let api = FakeApi::start();
    let target = FakeApi::start();
    target.route("/user", Response::json(r#"{"login":"octocat"}"#));
    api.route(
        "/user",
        Response::redirect(&format!("{}/user?x=1", target.base_url())),
    );
    let err = client(&api).viewer(&dom(), &token()).unwrap_err();
    let expected_origin = target.base_url();
    assert_matches!(
        &err,
        ApiError::Redirect { path, origin } if path == "/user" && *origin == expected_origin
    );
    assert_eq!(target.requests().len(), 0);
    assert_one_line_without_token(&err);
}

#[test]
fn renamed_org_redirect_keeps_auth_and_returns_private_repos() {
    let api = FakeApi::start();
    // GitHub answers a renamed org with a 301 to its id-based path; only an
    // authenticated follow-up sees the private repos.
    api.route(
        "/orgs/old/repos",
        Response::status(301).header("Location", "/organizations/42/repos?type=all&per_page=100"),
    );
    api.route(
        "/organizations/42/repos",
        Response::json(&page(&[repo_json("new", "private-thing")])),
    );
    let org: Org = "old".parse().unwrap();
    let listing = client(&api)
        .org_repos(&dom(), &token(), &org, &mut |_| {})
        .unwrap();
    assert_eq!(
        listing.repos[0].slug.to_string(),
        "github.com/new/private-thing"
    );
    let followed = api.distinct_requests_to("/organizations/42/repos");
    assert_eq!(followed.len(), 1);
    let auth = format!("Bearer {TOKEN}");
    assert_eq!(followed[0].authorization(), Some(auth.as_str()));
}

#[test]
fn redirect_loop_stops_after_a_few_hops() {
    let api = FakeApi::start();
    // `/user` -> `?hop=1` -> ... -> `?hop=5` -> `?hop=5`: every hop has its
    // own target, so counting distinct targets is immune to EINTR retries.
    api.route("/user", Response::redirect("/user?hop=1"));
    for hop in 1..=5 {
        let next = format!("/user?hop={}", (hop + 1).min(5));
        api.route_param("/user", "hop", &hop.to_string(), Response::redirect(&next));
    }
    let err = client(&api).viewer(&dom(), &token()).unwrap_err();
    assert_matches!(&err, ApiError::Status { status: 302, .. });
    assert_eq!(api.distinct_requests_to("/user").len(), 6);
}

fn status_error(response: Response) -> ApiError {
    let api = FakeApi::start();
    api.route("/user", response);
    client(&api).viewer(&dom(), &token()).unwrap_err()
}

fn assert_one_line_without_token(err: &ApiError) {
    let text = err.to_string();
    assert!(!text.contains('\n'), "{text:?}");
    assert!(!text.contains(TOKEN), "{text:?}");
    assert!(!format!("{err:?}").contains(TOKEN));
}

#[test]
fn unauthorized_suggests_gh_auth_login() {
    let err = status_error(Response::status(401));
    assert_matches!(
        &err,
        ApiError::Status {
            status: 401,
            hint: StatusHint::Login { .. },
            ..
        }
    );
    assert_eq!(
        err.to_string(),
        "HTTP 401 from /user (run 'gh auth login --hostname github.com')"
    );
    assert_one_line_without_token(&err);
}

#[test]
fn rate_limited_forbidden_is_recognised() {
    let err = status_error(Response::status(403).header("x-ratelimit-remaining", "0"));
    assert_matches!(
        &err,
        ApiError::Status {
            status: 403,
            hint: StatusHint::RateLimit,
            ..
        }
    );
    assert_eq!(
        err.to_string(),
        "HTTP 403 from /user (API rate limit exceeded; try again later)"
    );
    assert_one_line_without_token(&err);

    let err = status_error(Response::status(429));
    assert_matches!(
        &err,
        ApiError::Status {
            hint: StatusHint::RateLimit,
            ..
        }
    );
}

#[test]
fn secondary_rate_limit_is_recognised_from_the_message() {
    let err = status_error(
        Response::json(r#"{"message":"You have exceeded a secondary rate limit. Please wait."}"#)
            .with_status(403),
    );
    assert_matches!(
        &err,
        ApiError::Status {
            status: 403,
            hint: StatusHint::RateLimit,
            ..
        }
    );
    assert!(!err.to_string().contains("secondary"));
}

#[test]
fn plain_forbidden_says_the_token_lacks_access() {
    let err = status_error(Response::status(403).header("x-ratelimit-remaining", "4999"));
    assert_matches!(
        &err,
        ApiError::Status {
            status: 403,
            hint: StatusHint::Forbidden,
            ..
        }
    );
    assert_eq!(
        err.to_string(),
        "HTTP 403 from /user (the token lacks access to this resource)"
    );
    assert_one_line_without_token(&err);
}

#[test]
fn sso_forbidden_suggests_authorizing_the_token() {
    let err = status_error(
        Response::status(403).header("X-GitHub-SSO", "required; url=https://github.com/x"),
    );
    assert_matches!(
        &err,
        ApiError::Status {
            status: 403,
            hint: StatusHint::Sso { .. },
            ..
        }
    );
    assert!(
        err.to_string()
            .contains("gh auth refresh --hostname github.com")
    );
    assert_one_line_without_token(&err);
}

#[test]
fn missing_org_says_it_is_not_visible() {
    let api = FakeApi::start();
    let org: Org = "ghost".parse().unwrap();
    let err = client(&api)
        .org_repos(&dom(), &token(), &org, &mut |_| {})
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "HTTP 404 from /orgs/ghost/repos (org not found or not visible to this token)"
    );
}

#[test]
fn server_error_is_one_line_with_the_path_and_no_query() {
    let api = FakeApi::start();
    api.route("/orgs/acme/repos", Response::status(502));
    let org: Org = "acme".parse().unwrap();
    let err = client(&api)
        .org_repos(&dom(), &token(), &org, &mut |_| {})
        .unwrap_err();
    assert_matches!(
        &err,
        ApiError::Status {
            status: 502,
            hint: StatusHint::ServerError,
            ..
        }
    );
    assert_eq!(
        err.to_string(),
        "HTTP 502 from /orgs/acme/repos (GitHub server error; try again later)"
    );
    assert_one_line_without_token(&err);
}

#[test]
fn invalid_json_is_a_decode_error() {
    let api = FakeApi::start();
    api.route("/user/repos", Response::json("not json"));
    let err = client(&api)
        .user_repos(&dom(), &token(), &mut |_| {})
        .unwrap_err();
    assert_matches!(&err, ApiError::Decode { path, .. } if path == "/user/repos");
    assert_one_line_without_token(&err);
}

#[test]
fn connection_failure_is_a_one_line_transport_error() {
    let api = FakeApi::start();
    let base = ApiUrl::try_from(api.base_url()).unwrap();
    drop(api);
    // Nothing listens on the dropped server's port (or something unrelated
    // does); either way the request cannot succeed as a 200 `/user`.
    let err = HttpGitHub::new(BTreeMap::from([(dom(), base)]))
        .viewer(&dom(), &token())
        .unwrap_err();
    assert_one_line_without_token(&err);
}

#[test]
fn invalid_entries_are_skipped_and_counted() {
    let api = FakeApi::start();
    let entries = [
        repo_json("acme", "good"),
        // clone_url not https.
        r#"{"name":"a","owner":{"login":"acme"},"clone_url":"file:///tmp/x.git","html_url":"https://github.com/acme/a"}"#.to_owned(),
        // clone_url on another host.
        r#"{"name":"b","owner":{"login":"acme"},"clone_url":"https://evil.com/acme/b.git","html_url":"https://github.com/acme/b"}"#.to_owned(),
        // html_url on another host.
        r#"{"name":"c","owner":{"login":"acme"},"clone_url":"https://github.com/acme/c.git","html_url":"https://evil.com/acme/c"}"#.to_owned(),
        // html_url not https.
        r#"{"name":"d","owner":{"login":"acme"},"clone_url":"https://github.com/acme/d.git","html_url":"http://github.com/acme/d"}"#.to_owned(),
        // Missing fields.
        r#"{"name":"e"}"#.to_owned(),
    ];
    api.route("/orgs/acme/repos", Response::json(&page(&entries)));
    let org: Org = "acme".parse().unwrap();
    let mut seen = Vec::new();
    let listing = client(&api)
        .org_repos(&dom(), &token(), &org, &mut |p| seen.push(p))
        .unwrap();
    assert_eq!(listing.repos.len(), 1);
    assert_eq!(listing.repos[0].slug.to_string(), "github.com/acme/good");
    assert_eq!(listing.skipped, 5);
    assert_eq!(
        seen,
        [Page {
            number: 1,
            repos_so_far: 1
        }]
    );
}

#[test]
fn listing_is_truncated_at_the_page_cap() {
    let api = FakeApi::start();
    // Page N links to page N + 1, forever; the last page served before the
    // cap links off-origin, which the cap reports as truncation, not an error.
    // Pages are routed by their `page` parameter (not served in sequence),
    // so a request the client repeats after an EINTR gets the same page.
    let base = api.base_url();
    for n in 1..=MAX_PAGES {
        let next = if n == MAX_PAGES {
            "https://elsewhere.invalid/user/repos?page=101".to_owned()
        } else {
            format!("{base}/user/repos?page={}", n + 1)
        };
        let response = Response::json(&page(&[repo_json("me", &format!("r{n}"))])).link_next(&next);
        if n == 1 {
            api.route("/user/repos", response);
        } else {
            api.route_param("/user/repos", "page", &n.to_string(), response);
        }
    }
    let mut pages = 0;
    let listing = client(&api)
        .user_repos(&dom(), &token(), &mut |_| pages += 1)
        .unwrap();
    assert!(listing.truncated);
    assert_eq!(pages, MAX_PAGES);
    let expected = usize::try_from(MAX_PAGES).unwrap();
    assert_eq!(listing.repos.len(), expected);
    assert_eq!(api.distinct_requests_to("/user/repos").len(), expected);
}

#[test]
fn a_next_link_back_to_a_fetched_page_is_a_loop_error() {
    let api = FakeApi::start();
    let base = api.base_url();
    api.route(
        "/user/repos",
        Response::json(&page(&[repo_json("me", "a")]))
            .link_next(&format!("{base}/user/repos?page=2")),
    );
    api.route_param(
        "/user/repos",
        "page",
        "2",
        Response::json(&page(&[repo_json("me", "b")]))
            .link_next(&format!("{base}/user/repos?page=2")),
    );
    let err = client(&api)
        .user_repos(&dom(), &token(), &mut |_| {})
        .unwrap_err();
    assert_matches!(&err, ApiError::PaginationLoop { path } if path == "/user/repos");
    assert_eq!(api.distinct_requests_to("/user/repos").len(), 2);
}
