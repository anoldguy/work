//! The environment each git child runs with.

use std::ffi::OsString;
use std::fmt;
use std::process::Command;

use crate::auth::GitAuth;
use crate::env::GitConfigCount;

/// Environment changes for one git child.
///
/// Always sets `LC_ALL=C` (git's messages are never localised),
/// `GIT_TERMINAL_PROMPT=0` and `GIT_OPTIONAL_LOCKS=0` (status never takes
/// `index.lock` under the user's editor), and removes the trace variables
/// and the repository-locating variables (`GIT_DIR` and friends, so
/// `-C <repo>` means what it says).
///
/// After the user's existing `GIT_CONFIG_KEY_n`/`VALUE_n` entries (raising
/// `GIT_CONFIG_COUNT`) it appends `protocol.ext.allow=never`, which keeps
/// `ext::` blocked even if user config allows it. git's other protocol
/// defaults (such as `protocol.file.allow=user`) are left alone. With auth
/// it also appends, for `https://<host>/` only:
///
/// - an empty `http.<url>.extraHeader`, which resets any header already in
///   config (for example one persisted by `actions/checkout`), so exactly one
///   `Authorization` header is sent;
/// - our `Authorization: Basic …` header;
/// - `http.<url>.followRedirects=false`, because git rebases later requests
///   onto a redirect target and would send the header there.
///
/// [`Domain`](crate::domain::Domain) has no port, so a server on a
/// non-default port (`https://host:8443/`) does not match the URL key and
/// gets no header. That fails safe.
///
/// `Debug` prints variable names only, never values.
pub struct GitEnv {
    /// Variables to set.
    pub set: Vec<(OsString, OsString)>,
    /// Variables to remove.
    pub remove: Vec<OsString>,
}

impl GitEnv {
    /// Applies the changes to `cmd`.
    pub fn apply(&self, cmd: &mut Command) {
        for name in &self.remove {
            cmd.env_remove(name);
        }
        cmd.envs(self.set.iter().map(|(k, v)| (k, v)));
    }
}

impl fmt::Debug for GitEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitEnv")
            .field("set", &self.set.iter().map(|(k, _)| k).collect::<Vec<_>>())
            .field("remove", &self.remove)
            .finish()
    }
}

/// Trace variables removed even when they were not in the snapshot.
const TRACE_VARS: &[&str] = &[
    "GIT_TRACE",
    "GIT_TRACE_CURL",
    "GIT_TRACE_CURL_NO_DATA",
    "GIT_TRACE_PACKET",
    "GIT_TRACE_PACKFILE",
    "GIT_TRACE_PERFORMANCE",
    "GIT_TRACE_SETUP",
    "GIT_TRACE_SHALLOW",
    "GIT_TRACE2",
    "GIT_TRACE2_EVENT",
    "GIT_TRACE2_PERF",
    "GIT_CURL_VERBOSE",
];

/// Variables that would point git at a different repository than `-C`.
const REPO_VARS: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
];

/// Builds the environment for one git child. `config_count` is the user's
/// `GIT_CONFIG_COUNT` (with [`GitConfigCount::Invalid`] no config entries,
/// and so no auth, are added); `trace_vars` are the inherited trace
/// variable names.
#[must_use]
pub fn git_env(
    auth: Option<&GitAuth<'_>>,
    config_count: GitConfigCount,
    trace_vars: &[OsString],
) -> GitEnv {
    let scoped = auth.map(|auth| (format!("https://{}", auth.host()), auth));
    git_env_scoped(
        scoped.as_ref().map(|(base, auth)| (base.as_str(), *auth)),
        config_count,
        trace_vars,
    )
}

/// [`git_env`] with the URL the header is scoped to (`scheme://host[:port]`
/// without a trailing slash) given explicitly, so tests can aim it at a
/// loopback server.
fn git_env_scoped(
    auth: Option<(&str, &GitAuth<'_>)>,
    config_count: GitConfigCount,
    trace_vars: &[OsString],
) -> GitEnv {
    let mut set: Vec<(OsString, OsString)> = [
        ("LC_ALL", "C"),
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_OPTIONAL_LOCKS", "0"),
    ]
    .into_iter()
    .map(|(k, v)| (OsString::from(k), OsString::from(v)))
    .collect();
    set.extend(config_entries(auth, config_count));
    let mut remove: Vec<OsString> = TRACE_VARS
        .iter()
        .chain(REPO_VARS)
        .map(OsString::from)
        .chain(trace_vars.iter().cloned())
        .collect();
    remove.sort();
    remove.dedup();
    GitEnv { set, remove }
}

/// The `GIT_CONFIG_KEY_n`/`VALUE_n` pairs after the user's own, plus the
/// raised `GIT_CONFIG_COUNT`; nothing when the existing count is invalid
/// (or would overflow).
fn config_entries(
    auth: Option<(&str, &GitAuth<'_>)>,
    config_count: GitConfigCount,
) -> Vec<(OsString, OsString)> {
    let start = match config_count {
        GitConfigCount::Unset => 0,
        GitConfigCount::Count(n) => n,
        GitConfigCount::Invalid => return Vec::new(),
    };
    let mut pairs = vec![("protocol.ext.allow".to_owned(), "never".to_owned())];
    if let Some((base, auth)) = auth {
        let header = format!("http.{base}/.extraHeader");
        pairs.extend([
            (header.clone(), String::new()),
            (
                header,
                format!("Authorization: Basic {}", auth.header_b64().expose()),
            ),
            (format!("http.{base}/.followRedirects"), "false".to_owned()),
        ]);
    }
    let Some(count) = u32::try_from(pairs.len())
        .ok()
        .and_then(|n| start.checked_add(n))
    else {
        return Vec::new();
    };
    (start..)
        .zip(pairs)
        .flat_map(|(i, (key, value))| {
            [
                (
                    OsString::from(format!("GIT_CONFIG_KEY_{i}")),
                    OsString::from(key),
                ),
                (
                    OsString::from(format!("GIT_CONFIG_VALUE_{i}")),
                    OsString::from(value),
                ),
            ]
        })
        .chain([(
            OsString::from("GIT_CONFIG_COUNT"),
            OsString::from(count.to_string()),
        )])
        .collect()
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::ffi::{OsStr, OsString};
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::auth::SecretToken;
    use crate::domain::Domain;

    fn lookup<'a>(env: &'a GitEnv, name: &str) -> Option<&'a OsStr> {
        env.set
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_os_str())
    }

    fn with_auth<R>(f: impl FnOnce(&GitAuth<'_>) -> R) -> R {
        let token = SecretToken::new("gho_envsecret").expect("valid");
        let host = "ghe.example.com".parse::<Domain>().expect("valid domain");
        f(&GitAuth::new(host, &token))
    }

    #[test]
    fn always_sets_locale_prompt_locks_and_ext_block() {
        let env = git_env(None, GitConfigCount::Unset, &[]);
        assert_eq!(lookup(&env, "LC_ALL"), Some(OsStr::new("C")));
        assert_eq!(lookup(&env, "GIT_TERMINAL_PROMPT"), Some(OsStr::new("0")));
        assert_eq!(lookup(&env, "GIT_OPTIONAL_LOCKS"), Some(OsStr::new("0")));
        assert_matches!(lookup(&env, "GIT_ALLOW_PROTOCOL"), None);
        assert_eq!(lookup(&env, "GIT_CONFIG_COUNT"), Some(OsStr::new("1")));
        assert_eq!(
            lookup(&env, "GIT_CONFIG_KEY_0"),
            Some(OsStr::new("protocol.ext.allow"))
        );
        assert_eq!(
            lookup(&env, "GIT_CONFIG_VALUE_0"),
            Some(OsStr::new("never"))
        );
    }

    #[test]
    fn auth_resets_then_sets_header_and_disables_redirects() {
        with_auth(|auth| {
            let env = git_env(Some(auth), GitConfigCount::Unset, &[]);
            let key = OsStr::new("http.https://ghe.example.com/.extraHeader");
            assert_eq!(lookup(&env, "GIT_CONFIG_COUNT"), Some(OsStr::new("4")));
            assert_eq!(lookup(&env, "GIT_CONFIG_KEY_1"), Some(key));
            assert_eq!(lookup(&env, "GIT_CONFIG_VALUE_1"), Some(OsStr::new("")));
            assert_eq!(lookup(&env, "GIT_CONFIG_KEY_2"), Some(key));
            let value = format!("Authorization: Basic {}", auth.header_b64().expose());
            assert_eq!(lookup(&env, "GIT_CONFIG_VALUE_2"), Some(OsStr::new(&value)));
            assert_eq!(
                lookup(&env, "GIT_CONFIG_KEY_3"),
                Some(OsStr::new("http.https://ghe.example.com/.followRedirects"))
            );
            assert_eq!(
                lookup(&env, "GIT_CONFIG_VALUE_3"),
                Some(OsStr::new("false"))
            );
        });
    }

    #[test]
    fn auth_appends_after_existing_entries() {
        with_auth(|auth| {
            let env = git_env(Some(auth), GitConfigCount::Count(3), &[]);
            assert_eq!(lookup(&env, "GIT_CONFIG_COUNT"), Some(OsStr::new("7")));
            assert_matches!(lookup(&env, "GIT_CONFIG_KEY_3"), Some(_));
            assert_matches!(lookup(&env, "GIT_CONFIG_VALUE_6"), Some(_));
            assert_matches!(lookup(&env, "GIT_CONFIG_KEY_0"), None);
        });
    }

    #[test]
    fn invalid_or_saturated_count_adds_no_config() {
        with_auth(|auth| {
            for count in [
                GitConfigCount::Invalid,
                GitConfigCount::Count(u32::MAX),
                GitConfigCount::Count(u32::MAX - 2),
            ] {
                let env = git_env(Some(auth), count, &[]);
                assert!(
                    env.set
                        .iter()
                        .all(|(k, _)| !k.to_string_lossy().starts_with("GIT_CONFIG")),
                    "{count:?}"
                );
            }
        });
    }

    #[test]
    fn strips_trace_and_repo_variables() {
        let env = git_env(
            None,
            GitConfigCount::Unset,
            &[OsString::from("GIT_TRACE_FOO")],
        );
        for name in ["GIT_TRACE_FOO", "GIT_TRACE", "GIT_CURL_VERBOSE", "GIT_DIR"] {
            assert!(env.remove.iter().any(|r| r == name), "{name}");
        }
    }

    #[test]
    fn debug_shows_names_not_values() {
        with_auth(|auth| {
            let env = git_env(Some(auth), GitConfigCount::Unset, &[]);
            let shown = format!("{env:?}");
            assert!(shown.contains("GIT_CONFIG_VALUE_2"), "{shown}");
            assert!(!shown.contains("gho_envsecret"), "{shown}");
            assert!(!shown.contains(auth.header_b64().expose()), "{shown}");
            assert!(!shown.contains("Authorization"), "{shown}");
        });
    }

    /// Request lines with their `Authorization` values.
    type Requests = Vec<(String, Vec<String>)>;

    /// A loopback HTTP server that records the request line and every
    /// `Authorization` header of each request and answers with `respond`.
    struct Server {
        port: u16,
        log: Arc<Mutex<Requests>>,
    }

    impl Server {
        fn start(respond: impl Fn(&str) -> String + Send + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let port = listener.local_addr().expect("addr").port();
            let log = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&log);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                    let mut request = String::new();
                    let mut auths = Vec::new();
                    let mut line = String::new();
                    while reader.read_line(&mut line).is_ok_and(|n| n > 0) {
                        let trimmed = line.trim_end();
                        if trimmed.is_empty() {
                            break;
                        }
                        if request.is_empty() {
                            request = trimmed.to_owned();
                        } else if let Some((name, value)) = trimmed.split_once(':')
                            && name.eq_ignore_ascii_case("authorization")
                        {
                            auths.push(value.trim().to_owned());
                        }
                        line.clear();
                    }
                    let reply = respond(&request);
                    sink.lock().expect("log").push((request, auths));
                    let _ = stream.write_all(reply.as_bytes());
                }
            });
            Self { port, log }
        }

        fn requests(&self) -> Requests {
            self.log.lock().expect("log").clone()
        }
    }

    const NOT_FOUND: &str =
        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

    /// Runs `git <args>` in `dir` with our env aimed at `base`, isolated from
    /// the user's config. Returns success.
    fn run_git(dir: &Path, base: &str, auth: &GitAuth<'_>, args: &[&str]) -> bool {
        let mut cmd = Command::new("git");
        cmd.args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        git_env_scoped(Some((base, auth)), GitConfigCount::Unset, &[]).apply(&mut cmd);
        crate::proc::run_capture(&mut cmd)
            .expect("git runs")
            .status
            .success()
    }

    #[test]
    fn header_is_not_sent_to_a_redirect_target() {
        let target = Server::start(|_| NOT_FOUND.to_owned());
        let target_port = target.port;
        let origin = Server::start(move |_| {
            format!(
                "HTTP/1.1 301 Moved Permanently\r\nLocation: http://127.0.0.1:{target_port}/acme/foo.git/info/refs?service=git-upload-pack\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
        });
        let tmp = tempfile::tempdir().expect("tempdir");
        with_auth(|auth| {
            let base = format!("http://127.0.0.1:{}", origin.port);
            let url = format!("{base}/acme/foo.git");
            assert!(!run_git(tmp.path(), &base, auth, &["ls-remote", &url]));
            let expected = format!("Basic {}", auth.header_b64().expose());
            let first = origin.requests();
            assert_eq!(first.len(), 1, "{first:?}");
            assert_eq!(first[0].1, [expected], "header goes to its own host");
            assert_eq!(target.requests(), [], "redirect target never contacted");
        });
    }

    #[test]
    fn header_already_in_repo_config_is_replaced_not_duplicated() {
        let server = Server::start(|_| NOT_FOUND.to_owned());
        let tmp = tempfile::tempdir().expect("tempdir");
        let base = format!("http://127.0.0.1:{}", server.port);
        with_auth(|auth| {
            assert!(run_git(tmp.path(), &base, auth, &["init", "-q", "."]));
            let key = format!("http.{base}/.extraheader");
            assert!(run_git(
                tmp.path(),
                &base,
                auth,
                &["config", &key, "AUTHORIZATION: basic OLDCHECKOUT"],
            ));
            let url = format!("{base}/acme/foo.git");
            assert!(!run_git(tmp.path(), &base, auth, &["ls-remote", &url]));
            let expected = format!("Basic {}", auth.header_b64().expose());
            let requests = server.requests();
            assert_ne!(requests.len(), 0);
            for (_, auths) in requests {
                assert_eq!(auths, std::slice::from_ref(&expected));
            }
        });
    }

    #[test]
    fn header_is_not_sent_to_another_port_on_the_same_host() {
        let other = Server::start(|_| NOT_FOUND.to_owned());
        let tmp = tempfile::tempdir().expect("tempdir");
        with_auth(|auth| {
            // Scoped to port 1, used against `other`'s port.
            let url = format!("http://127.0.0.1:{}/acme/foo.git", other.port);
            assert!(!run_git(
                tmp.path(),
                "http://127.0.0.1:1",
                auth,
                &["ls-remote", &url]
            ));
            let requests = other.requests();
            assert_ne!(requests.len(), 0);
            assert!(requests.iter().all(|(_, auths)| auths.is_empty()));
        });
    }
}
