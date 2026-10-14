//! A fake GitHub REST API: HTTP/1.1 on `127.0.0.1`, served from a std
//! `TcpListener` in a background thread.
//!
//! Routes are registered on a running server and matched on method `GET`,
//! path and optionally one query parameter (most specific wins, later
//! registrations override earlier ones). Unrouted requests get a 404. Every
//! request is recorded, headers included, so tests can assert on token
//! scoping. Dropping the server stops it: the drop sets a flag and connects
//! to the listener once to wake `accept`, so there are no timing waits.

use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// A canned HTTP response.
#[derive(Debug, Clone)]
pub struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Response {
    /// `200 OK` with a JSON body.
    #[must_use]
    pub fn json(body: &str) -> Self {
        Self {
            status: 200,
            headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
            body: body.to_owned(),
        }
    }

    /// An error-style response: `{"message": "<reason>"}` with `status`.
    #[must_use]
    pub fn status(status: u16) -> Self {
        let mut response = Self::json(&format!(r#"{{"message":"{}"}}"#, reason(status)));
        response.status = status;
        response
    }

    /// A `302 Found` to `location`.
    #[must_use]
    pub fn redirect(location: &str) -> Self {
        Self::status(302).header("Location", location)
    }

    /// Adds a response header.
    #[must_use]
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    /// Adds `Link: <next>; rel="next"`; `next` may be any (even off-origin)
    /// URL.
    #[must_use]
    pub fn link_next(self, next: &str) -> Self {
        self.header("Link", &format!(r#"<{next}>; rel="next""#))
    }

    /// Overrides the status code, keeping the body.
    #[must_use]
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        302 => "Found",
        401 => "Bad credentials",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

/// One request the server received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedRequest {
    /// HTTP method.
    pub method: String,
    /// Path without the query string.
    pub path: String,
    /// Raw query string (empty when none).
    pub query: String,
    /// Headers with lower-cased names, in arrival order.
    pub headers: Vec<(String, String)>,
}

impl RecordedRequest {
    /// The value of header `name` (case-insensitive), if sent.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }

    /// The `Authorization` header, if sent.
    #[must_use]
    pub fn authorization(&self) -> Option<&str> {
        self.header("authorization")
    }

    /// The value of query parameter `name`, if present.
    #[must_use]
    pub fn query_param(&self, name: &str) -> Option<String> {
        query_param(&self.query, name)
    }
}

/// The percent-decoded value of `name` in a raw query string.
fn query_param(query: &str, name: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| percent_decode(key) == name)
        .map(|(_, value)| percent_decode(value))
}

fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = bytes
            .get(index + 1..index + 3)
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (bytes[index], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                index += 3;
            }
            (b'+', _) => {
                out.push(b' ');
                index += 1;
            }
            (byte, _) => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[derive(Debug)]
struct Route {
    path: String,
    param: Option<(String, String)>,
    /// Served in order; the last one repeats.
    responses: Vec<Response>,
    hits: usize,
}

#[derive(Debug, Default)]
struct Shared {
    routes: Mutex<Vec<Route>>,
    requests: Mutex<Vec<RecordedRequest>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The running fake server. Stops when dropped.
#[derive(Debug)]
pub struct FakeApi {
    addr: SocketAddr,
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    /// Set by the accept thread as its last act.
    finished: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl FakeApi {
    /// Binds `127.0.0.1:0` and starts serving. The socket is bound before
    /// this returns, so clients may connect immediately.
    #[must_use]
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let shared = Arc::new(Shared::default());
        let stop = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let handle = {
            let shared = Arc::clone(&shared);
            let stop = Arc::clone(&stop);
            let finished = Arc::clone(&finished);
            thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let shared = Arc::clone(&shared);
                    thread::spawn(move || serve(stream, &shared));
                }
                finished.store(true, Ordering::SeqCst);
            })
        };
        Self {
            addr,
            shared,
            stop,
            finished,
            handle: Some(handle),
        }
    }

    /// `http://127.0.0.1:<port>`, the value for a domain's `api_url`.
    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// `http://127.0.0.1:<port>/<host>`: the per-domain API base that
    /// `DomainCfg` seeds as `api_url`, so each domain has its own routes.
    #[must_use]
    pub fn domain_url(&self, host: &str) -> String {
        format!("{}/{host}", self.base_url())
    }

    /// A handle whose routes and recorded requests are scoped to `host`'s
    /// own namespace (`/<host>/...`), served at [`FakeApi::domain_url`].
    #[must_use]
    pub fn host(&self, host: &str) -> HostApi<'_> {
        HostApi {
            api: self,
            prefix: format!("/{host}"),
        }
    }

    /// Serves `response` for `GET path` (any query).
    pub fn route(&self, path: &str, response: Response) {
        self.add(path, None, vec![response]);
    }

    /// Serves `responses` for successive `GET path` requests, then keeps
    /// repeating the last one (e.g. a 502 followed by a 200).
    ///
    /// # Panics
    ///
    /// Panics when `responses` is empty.
    pub fn route_seq(&self, path: &str, responses: Vec<Response>) {
        assert!(!responses.is_empty(), "route_seq needs a response");
        self.add(path, None, responses);
    }

    /// Serves `response` for `GET path` when the query has `key=value`.
    /// Takes precedence over plain [`FakeApi::route`] registrations.
    pub fn route_param(&self, path: &str, key: &str, value: &str, response: Response) {
        self.add(
            path,
            Some((key.to_owned(), value.to_owned())),
            vec![response],
        );
    }

    /// Serves JSON `pages` at `path`: page 1 for any request, page `N` for
    /// `?page=N`, each with a same-origin `Link: rel="next"` to the following
    /// page except the last.
    pub fn paged(&self, path: &str, pages: &[&str]) {
        for (index, body) in pages.iter().enumerate() {
            let number = index + 1;
            let mut response = Response::json(body);
            if number < pages.len() {
                let next = format!("{}{path}?page={}", self.base_url(), number + 1);
                response = response.link_next(&next);
            }
            if number == 1 {
                self.route(path, response);
            } else {
                self.route_param(path, "page", &number.to_string(), response);
            }
        }
    }

    fn add(&self, path: &str, param: Option<(String, String)>, responses: Vec<Response>) {
        lock(&self.shared.routes).push(Route {
            path: path.to_owned(),
            param,
            responses,
            hits: 0,
        });
    }

    /// Every request received so far, oldest first.
    #[must_use]
    pub fn requests(&self) -> Vec<RecordedRequest> {
        lock(&self.shared.requests).clone()
    }

    /// The requests whose path equals `path`.
    #[must_use]
    pub fn requests_to(&self, path: &str) -> Vec<RecordedRequest> {
        self.requests()
            .into_iter()
            .filter(|request| request.path == path)
            .collect()
    }
}

/// Routes and request log for one domain; see [`FakeApi::host`].
#[derive(Debug)]
pub struct HostApi<'a> {
    api: &'a FakeApi,
    prefix: String,
}

impl HostApi<'_> {
    /// [`FakeApi::route`] under this host.
    pub fn route(&self, path: &str, response: Response) {
        self.api.route(&format!("{}{path}", self.prefix), response);
    }

    /// [`FakeApi::route_seq`] under this host.
    pub fn route_seq(&self, path: &str, responses: Vec<Response>) {
        self.api
            .route_seq(&format!("{}{path}", self.prefix), responses);
    }

    /// [`FakeApi::route_param`] under this host.
    pub fn route_param(&self, path: &str, key: &str, value: &str, response: Response) {
        self.api
            .route_param(&format!("{}{path}", self.prefix), key, value, response);
    }

    /// [`FakeApi::paged`] under this host.
    pub fn paged(&self, path: &str, pages: &[&str]) {
        self.api.paged(&format!("{}{path}", self.prefix), pages);
    }

    /// This host's requests, oldest first, with the host prefix stripped
    /// from `path`.
    #[must_use]
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.api
            .requests()
            .into_iter()
            .filter_map(|mut request| {
                let stripped = request.path.strip_prefix(&self.prefix)?;
                if !stripped.starts_with('/') {
                    return None;
                }
                request.path = stripped.to_owned();
                Some(request)
            })
            .collect()
    }

    /// This host's requests whose (stripped) path equals `path`.
    #[must_use]
    pub fn requests_to(&self, path: &str) -> Vec<RecordedRequest> {
        self.requests()
            .into_iter()
            .filter(|request| request.path == path)
            .collect()
    }
}

impl Drop for FakeApi {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the blocked `accept`; the loop then sees the flag and exits.
        drop(TcpStream::connect(self.addr));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve(stream: TcpStream, shared: &Shared) {
    // A safety net against a stuck peer, not a synchronisation mechanism.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let Some(request) = read_request(&mut BufReader::new(stream)) else {
        return;
    };
    let response = lookup(shared, &request);
    lock(&shared.requests).push(request);
    let _ = write_response(&mut writer, &response);
}

fn read_request(reader: &mut BufReader<TcpStream>) -> Option<RecordedRequest> {
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let target = parts.next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let (path, query) = (path.to_owned(), query.to_owned());
    let mut headers = Vec::new();
    loop {
        line.clear();
        reader.read_line(&mut line).ok()?;
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        let (name, value) = trimmed.split_once(':')?;
        headers.push((name.to_ascii_lowercase(), value.trim().to_owned()));
    }
    let length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse::<u64>().ok())
        .unwrap_or(0);
    std::io::copy(&mut reader.take(length), &mut std::io::sink()).ok()?;
    Some(RecordedRequest {
        method,
        path,
        query,
        headers,
    })
}

fn lookup(shared: &Shared, request: &RecordedRequest) -> Response {
    if request.method != "GET" {
        return Response::status(404);
    }
    let mut routes = lock(&shared.routes);
    let on_path = |route: &Route| route.path == request.path;
    let index = routes
        .iter()
        .rposition(|route| {
            on_path(route)
                && route.param.as_ref().is_some_and(|(key, value)| {
                    request.query_param(key).as_deref() == Some(value.as_str())
                })
        })
        .or_else(|| {
            routes
                .iter()
                .rposition(|route| on_path(route) && route.param.is_none())
        });
    let Some(index) = index else {
        return Response::status(404);
    };
    let route = &mut routes[index];
    let pick = route.hits.min(route.responses.len() - 1);
    route.hits = route.hits.saturating_add(1);
    route.responses[pick].clone()
}

fn write_response(writer: &mut TcpStream, response: &Response) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        response.status,
        reason(response.status),
        response.body.len()
    );
    for (name, value) in &response.headers {
        let _ = write!(head, "{name}: {value}\r\n");
    }
    head.push_str("\r\n");
    writer.write_all(head.as_bytes())?;
    writer.write_all(response.body.as_bytes())?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal blocking client: returns (status line, headers, body).
    fn get(api: &FakeApi, target: &str, extra: &[(&str, &str)]) -> (String, String, String) {
        let mut stream = TcpStream::connect(api.addr).expect("connect");
        let mut request = format!("GET {target} HTTP/1.1\r\nHost: x\r\n");
        for (name, value) in extra {
            let _ = write!(request, "{name}: {value}\r\n");
        }
        request.push_str("\r\n");
        stream.write_all(request.as_bytes()).expect("send");
        let mut raw = String::new();
        stream.read_to_string(&mut raw).expect("read");
        let (head, body) = raw.split_once("\r\n\r\n").expect("response framing");
        let (status, headers) = head.split_once("\r\n").unwrap_or((head, ""));
        (status.to_owned(), headers.to_owned(), body.to_owned())
    }

    #[test]
    fn serves_routes_and_404s_the_rest() {
        let api = FakeApi::start();
        api.route("/user", Response::json(r#"{"login":"me"}"#));
        let (status, _, body) = get(&api, "/user?x=1", &[]);
        assert_eq!(status, "HTTP/1.1 200 OK");
        assert_eq!(body, r#"{"login":"me"}"#);
        let (status, _, _) = get(&api, "/nope", &[]);
        assert_eq!(status, "HTTP/1.1 404 Not Found");
    }

    #[test]
    fn status_codes_and_headers_are_served() {
        let api = FakeApi::start();
        api.route(
            "/limited",
            Response::status(403).header("x-ratelimit-remaining", "0"),
        );
        api.route("/moved", Response::redirect("http://other.invalid/x"));
        let (status, headers, body) = get(&api, "/limited", &[]);
        assert_eq!(status, "HTTP/1.1 403 Forbidden");
        assert!(headers.contains("x-ratelimit-remaining: 0"));
        assert_eq!(body, r#"{"message":"Forbidden"}"#);
        let (status, headers, _) = get(&api, "/moved", &[]);
        assert_eq!(status, "HTTP/1.1 302 Found");
        assert!(headers.contains("Location: http://other.invalid/x"));
    }

    #[test]
    fn later_routes_override_and_param_routes_win() {
        let api = FakeApi::start();
        api.route("/a", Response::json("1"));
        api.route("/a", Response::json("2"));
        api.route_param("/a", "k", "v", Response::json("3"));
        assert_eq!(get(&api, "/a", &[]).2, "2");
        assert_eq!(get(&api, "/a?k=v", &[]).2, "3");
        assert_eq!(get(&api, "/a?k=w", &[]).2, "2");
        api.route("/b", Response::json("ok").with_status(201));
        assert_eq!(get(&api, "/b", &[]).0, "HTTP/1.1 201 Status");
    }

    #[test]
    fn paged_emits_link_next_until_the_last_page() {
        let api = FakeApi::start();
        api.paged("/user/repos", &["[1]", "[2]", "[3]"]);
        let base = api.base_url();

        let (_, headers, body) = get(&api, "/user/repos?per_page=100", &[]);
        assert_eq!(body, "[1]");
        assert!(headers.contains(&format!(r#"Link: <{base}/user/repos?page=2>; rel="next""#)));

        let (_, headers, body) = get(&api, "/user/repos?page=2", &[]);
        assert_eq!(body, "[2]");
        assert!(headers.contains("page=3"));

        let (_, headers, body) = get(&api, "/user/repos?page=3", &[]);
        assert_eq!(body, "[3]");
        assert!(!headers.contains("Link"));
    }

    #[test]
    fn link_next_can_point_off_origin() {
        let api = FakeApi::start();
        api.route(
            "/x",
            Response::json("[]").link_next("http://evil.invalid/steal"),
        );
        let (_, headers, _) = get(&api, "/x", &[]);
        assert!(headers.contains(r#"<http://evil.invalid/steal>; rel="next""#));
    }

    #[test]
    fn records_requests_with_authorization() {
        let api = FakeApi::start();
        api.route("/user", Response::json("{}"));
        get(&api, "/user?a=b&page=2", &[("Authorization", "Bearer tok")]);
        get(&api, "/other", &[]);
        let all = api.requests();
        assert_eq!(all.len(), 2);
        let user = api.requests_to("/user");
        assert_eq!(user.len(), 1);
        assert_eq!(user[0].method, "GET");
        assert_eq!(user[0].authorization(), Some("Bearer tok"));
        assert_eq!(user[0].header("AUTHORIZATION"), Some("Bearer tok"));
        assert_eq!(user[0].query_param("page").as_deref(), Some("2"));
        assert_eq!(user[0].query_param("zzz"), None);
        assert_eq!(all[1].authorization(), None);
    }

    #[test]
    fn drop_stops_the_server() {
        let api = FakeApi::start();
        assert!(api.base_url().starts_with("http://127.0.0.1:"));
        let finished = Arc::clone(&api.finished);
        assert!(!finished.load(Ordering::SeqCst));
        // Drop joins the accept thread, so the flag is set once it returns.
        // (Re-connecting to the port proves nothing: a parallel test's
        // server may have re-bound it.)
        drop(api);
        assert!(finished.load(Ordering::SeqCst));
    }

    #[test]
    fn route_seq_serves_in_order_then_repeats_the_last() {
        let api = FakeApi::start();
        api.route_seq("/flaky", vec![Response::status(502), Response::json("ok")]);
        assert_eq!(get(&api, "/flaky", &[]).0, "HTTP/1.1 502 Bad Gateway");
        assert_eq!(get(&api, "/flaky", &[]).2, "ok");
        assert_eq!(get(&api, "/flaky", &[]).2, "ok");
    }

    #[test]
    fn query_values_are_percent_decoded() {
        let api = FakeApi::start();
        api.route("/user/repos", Response::json("[]"));
        get(
            &api,
            "/user/repos?affiliation=owner%2Ccollaborator&q=a+b",
            &[],
        );
        let request = &api.requests()[0];
        assert_eq!(
            request.query_param("affiliation").as_deref(),
            Some("owner,collaborator")
        );
        assert_eq!(request.query_param("q").as_deref(), Some("a b"));
    }

    #[test]
    fn hosts_have_separate_routes_and_request_logs() {
        let api = FakeApi::start();
        let (com, ghe) = (api.host("github.com"), api.host("ghe.test"));
        com.route("/user", Response::json(r#"{"login":"com"}"#));
        ghe.route("/user", Response::json(r#"{"login":"ghe"}"#));
        ghe.paged("/user/orgs", &["[1]", "[2]"]);
        let get_host = |host: &str, path: &str| get(&api, &format!("/{host}{path}"), &[]).2;
        assert_eq!(get_host("github.com", "/user"), r#"{"login":"com"}"#);
        assert_eq!(get_host("ghe.test", "/user"), r#"{"login":"ghe"}"#);
        assert_eq!(get_host("ghe.test", "/user/orgs"), "[1]");
        let (_, headers, body) = get(&api, "/ghe.test/user/orgs?page=2", &[]);
        assert_eq!(body, "[2]");
        assert!(!headers.contains("Link"));
        com.route_seq("/seq", vec![Response::status(502), Response::json("up")]);
        com.route_param("/seq", "x", "1", Response::json("param"));
        assert_eq!(
            get_host("github.com", "/seq"),
            r#"{"message":"Bad Gateway"}"#
        );
        assert_eq!(get_host("github.com", "/seq"), "up");
        assert_eq!(get_host("github.com", "/seq?x=1"), "param");
        assert_eq!(com.requests().len(), 4);
        assert_eq!(ghe.requests_to("/user/orgs").len(), 2);
        assert_eq!(ghe.requests()[0].path, "/user");
        assert_eq!(
            api.domain_url("ghe.test"),
            format!("{}/ghe.test", api.base_url())
        );
    }
}
