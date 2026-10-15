//! End-to-end invariants (W6.1), table-driven over every command.
//!
//! Each `#[test]` below drives one failure mode (bad flag, missing argument,
//! missing/invalid config, missing/corrupt cache, ambiguous query, no match,
//! API failure, missing `gh`) through every command it applies to, and
//! checks the cross-cutting rules from PLAN §6.1 and AGENTS.md:
//!
//! 1. A failure writes exactly one stderr line starting with `work: ` (a
//!    list error follows it with only `  - <item>` lines), and it is the
//!    last line apart from those items. Every other stderr line, on failure
//!    and on success alike, is a `warning: ` line: per
//!    `specs/repository-cache.md`, per-domain and per-org refresh failures
//!    warn even when every domain fails and the command then errors. One
//!    helper, [`assert_stderr_shape`], enforces this for every run.
//! 2. A failure writes nothing to stdout.
//! 3. `sh-cd`/`sh-on` print exactly one line of script on success and
//!    nothing on failure.
//! 4. The `GITHUB_TOKEN` (and the token `gh` hands out) never shows up raw
//!    or base64-encoded in stdout, stderr, any file under the test root
//!    (cache, config, clones), or the shim call logs.
//! 5. Exit codes: 0, 1, 2 for usage errors. Exit 130 needs Ctrl-C in the
//!    picker, which needs a terminal; it is covered by `error.rs` unit
//!    tests, not here. A broken stdout pipe is silent and exits 0.
//! 6. `--help` of `work` and every visible subcommand is snapshotted, and
//!    hidden commands stay out of the top-level help.
//!
//! Every env sets `GITHUB_TOKEN` (or the `gh` token) to [`SENTINEL`], so
//! rule 4 is checked on every single run, successes included.

use std::fs;
use std::path::Path;
use std::process::{Output, Stdio};

use crate::support::{DomainCfg, RepoSeed, TestEnv, repo_line};

/// The fake token every env carries; it must never leak.
const SENTINEL: &str = "ghp_SENTINEL0123456789abcdefghijklmnopqrst";

/// A query that matches nothing in any fixture.
const NO_MATCH: &str = "zzzz-nothing";

/// Every way the sentinel could leak: raw, the git `extraHeader` credential
/// (`base64("x-access-token:" + token)`), and plain base64 of the token.
fn forbidden() -> [String; 3] {
    [
        SENTINEL.to_owned(),
        base64(format!("x-access-token:{SENTINEL}").as_bytes()),
        base64(SENTINEL.as_bytes()),
    ]
}

/// RFC 4648 base64 with padding (the tests' own copy, independent of the
/// crate's).
fn base64(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let bytes = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                let index = usize::try_from((n >> (18 - 6 * i)) & 63).expect("6 bits");
                out.push(char::from(ALPHABET[index]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The state a command runs against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fixture {
    /// No config, no cache, nothing routed: for usage errors.
    Bare,
    /// github.com config, a healthy API, a fresh cache holding `acme/foo`
    /// (cloned) and `acme/bar` (not cloned), both from local origins.
    Ready,
    /// Like `Ready`, but `GITHUB_TOKEN` is unset and `gh` supplies the
    /// sentinel instead.
    GhToken,
    /// No config file (implicit github.com) but a fresh cache, so nothing
    /// needs the network.
    NoConfig,
    /// `config.toml` is not valid TOML.
    BadConfig,
    /// `config.toml` parses but breaks a rule (an uppercase domain).
    InvalidConfig,
    /// Config, no cache, and the API fails.
    NoCache,
    /// Config, an unparsable cache, and the API fails.
    CorruptCache,
    /// Config, a fresh cache with `acme/foo-api` and `acme/foo-web`.
    Ambiguous,
    /// `Ready` with every API route failing (the error body echoes the
    /// token).
    ApiDown,
    /// `Ready` with a stale cache and every API route failing.
    StaleApiDown,
    /// Config, no cache, no `gh` on `PATH`, no `GITHUB_TOKEN`.
    NoGh,
}

/// What a run must look like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Want {
    /// Exit 0; stderr holds only `warning: ` lines.
    Ok,
    /// Exit 0; stderr carries exactly this informational output (by design,
    /// keeping stdout clean for scripts) and nothing else.
    OkWithStderr(Info),
    /// Exit 2, one `work: ` line.
    Usage,
    /// Exit 1, one `work: ` line.
    Error,
    /// Exit 1, one `work: ` line plus exactly this many `  - ` items.
    List(usize),
}

/// The informational stderr a successful command writes on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Info {
    /// Exactly this text, after the test home is replaced by `<HOME>`:
    /// `config path` (`config file exists|missing`), `config default <d>`
    /// (`default domain is now <d>`), `completions <sh> --install`.
    Exact(&'static str),
    /// `about` without a terminal: the static logo and footer, compared
    /// against the `about_static_logo` snapshot (version redacted).
    AboutLogo,
}

/// One row of a table: `work <args>` in `fixture` must give `want`.
#[derive(Debug, Clone)]
struct Case {
    fixture: Fixture,
    args: Vec<&'static str>,
    want: Want,
}

fn case(fixture: Fixture, args: &[&'static str], want: Want) -> Case {
    Case {
        fixture,
        args: args.to_vec(),
        want,
    }
}

fn healthy_api(env: &TestEnv) {
    let gh = &env.gh;
    gh.set_api("github.com", "user", &["me".to_owned()]);
    gh.set_api("github.com", "user/orgs", &["acme".to_owned()]);
    gh.set_api(
        "github.com",
        "user/repos",
        &[repo_line("github.com", "me", "dotfiles")],
    );
    gh.set_api(
        "github.com",
        "orgs/acme/repos",
        &[
            repo_line("github.com", "acme", "foo"),
            repo_line("github.com", "acme", "bar"),
        ],
    );
}

/// Every `gh api` call fails the way gh reports a server error.
fn failing_api(env: &TestEnv) {
    for path in ["user", "user/orgs", "user/repos", "orgs/acme/repos"] {
        env.gh
            .fail_api("github.com", path, "gh: Server Error (HTTP 500)\n");
    }
}

fn github_config(env: &TestEnv) {
    env.config()
        .domain(DomainCfg::new("github.com").default_domain())
        .write();
}

/// `acme/foo` (cloned) and `acme/bar` (not cloned), from local origins.
fn foo_and_bar(env: &TestEnv) -> Vec<RepoSeed> {
    let cloned = env.origin("acme", "foo");
    let uncloned = env.origin("acme", "bar");
    env.clone_into_work_dir(&cloned, "github.com/acme/foo");
    vec![
        RepoSeed::new("github.com/acme/foo").origin(&cloned),
        RepoSeed::new("github.com/acme/bar").origin(&uncloned),
    ]
}

fn build(fixture: Fixture) -> TestEnv {
    let env = TestEnv::new().token(Some(SENTINEL));
    env.gh.set_token("github.com", SENTINEL);
    match fixture {
        Fixture::Bare => {}
        Fixture::Ready => {
            github_config(&env);
            healthy_api(&env);
            let repos = foo_and_bar(&env);
            env.seed_cache(&repos);
        }
        Fixture::GhToken => {
            let env = env.token(None);
            github_config(&env);
            healthy_api(&env);
            let repos = foo_and_bar(&env);
            env.seed_cache(&repos);
            return env;
        }
        Fixture::NoConfig => {
            // No `config()` call: `repo_path` falls back to the full slug,
            // so seed the cache by hand with the implicit-default layout.
            let origin = env.origin("acme", "foo");
            origin.clone_to(&env.work_dir().join("acme/foo"));
            env.seed_cache(&[RepoSeed::new("github.com/acme/foo").origin(&origin)]);
        }
        Fixture::BadConfig => {
            env.write_config_raw("work_dir = [unterminated\n");
            env.seed_cache(&[RepoSeed::new("github.com/acme/foo")]);
        }
        Fixture::InvalidConfig => {
            env.write_config_raw("[domains.\"GitHub.com\"]\nis_default = true\n");
            env.seed_cache(&[RepoSeed::new("github.com/acme/foo")]);
        }
        Fixture::NoCache => {
            github_config(&env);
            failing_api(&env);
        }
        Fixture::CorruptCache => {
            github_config(&env);
            failing_api(&env);
            fs::create_dir_all(env.cache_path().parent().expect("parent")).expect("mkdir");
            fs::write(env.cache_path(), "{ not json").expect("write cache");
        }
        Fixture::Ambiguous => {
            github_config(&env);
            healthy_api(&env);
            env.seed_cache(&[
                RepoSeed::new("github.com/acme/foo-api"),
                RepoSeed::new("github.com/acme/foo-web"),
            ]);
        }
        Fixture::ApiDown => {
            github_config(&env);
            failing_api(&env);
            let repos = foo_and_bar(&env);
            env.seed_cache(&repos);
        }
        Fixture::StaleApiDown => {
            github_config(&env);
            failing_api(&env);
            let repos = foo_and_bar(&env);
            env.seed_stale_cache(&repos);
        }
        Fixture::NoGh => {
            let env = env.token(None);
            fs::remove_file(env.shim_dir().join("gh")).expect("remove gh shim");
            github_config(&env);
            failing_api(&env);
            return env;
        }
    }
    env
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Runs one case in a fresh env and checks every invariant that applies.
fn check(case: &Case) {
    let env = build(case.fixture);
    let out = env.command().args(&case.args).output().expect("run work");
    let ctx = format!("{case:?}");
    check_shape(&env, &out, case.want, &ctx);
    check_sh_script(&out, &case.args, &ctx);
    check_no_leak(&env, &out, &ctx);
}

/// Rules 1, 2 and 5: exit code, empty stdout on failure, and the stderr
/// shape via [`assert_stderr_shape`] (with the test home shown as
/// `<HOME>`).
fn check_shape(env: &TestEnv, out: &Output, want: Want, ctx: &str) {
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr).replace(&env.home().display().to_string(), "<HOME>");
    let expected = match want {
        Want::Ok | Want::OkWithStderr(_) => 0,
        Want::Usage => 2,
        Want::Error | Want::List(_) => 1,
    };
    assert_eq!(
        out.status.code(),
        Some(expected),
        "{ctx}: exit code\nstdout: {stdout:?}\nstderr: {stderr:?}"
    );
    if !matches!(want, Want::Ok | Want::OkWithStderr(_)) {
        assert_eq!(stdout, "", "{ctx}: a failure must not write to stdout");
    }
    assert_stderr_shape(&stderr, want, ctx);
}

/// The one stderr rule (rule 1). Stderr is, in order:
///
/// - any number of `warning: ` lines;
/// - on failure, exactly one `work: <message>` line;
/// - for a list error, exactly the expected number of `  - <item>` lines.
///
/// Nothing else, and every line ends in a newline. On success there is no
/// `work: ` line, so stderr is warnings only, except that
/// [`Want::OkWithStderr`] demands its exact informational text instead.
fn assert_stderr_shape(stderr: &str, want: Want, ctx: &str) {
    match want {
        Want::OkWithStderr(Info::Exact(expected)) => {
            assert_eq!(stderr, expected, "{ctx}: informational stderr");
            return;
        }
        Want::OkWithStderr(Info::AboutLogo) => {
            assert!(
                stderr
                    .lines()
                    .all(|line| !line.starts_with("work: ") && !line.starts_with("warning: ")),
                "{ctx}: the logo must not carry errors or warnings: {stderr:?}"
            );
            let version = format!("v{}", env!("CARGO_PKG_VERSION"));
            assert!(stderr.contains(&version), "{ctx}: no {version}: {stderr:?}");
            insta::assert_snapshot!("about_static_logo", stderr.replace(&version, "v<VERSION>"));
            return;
        }
        _ => {}
    }
    assert!(
        stderr.is_empty() || stderr.ends_with('\n'),
        "{ctx}: stderr must end with a newline: {stderr:?}"
    );
    let lines: Vec<&str> = stderr.lines().collect();
    let warnings = lines
        .iter()
        .take_while(|line| line.starts_with("warning: "))
        .count();
    let rest = &lines[warnings..];
    if want == Want::Ok {
        assert!(
            rest.is_empty(),
            "{ctx}: a success may only write `warning: ` lines to stderr: {stderr:?}"
        );
        return;
    }
    let Some((error, items)) = rest.split_first() else {
        panic!("{ctx}: a failure must write one `work: ` line to stderr: {stderr:?}");
    };
    let message = error.strip_prefix("work: ").unwrap_or_else(|| {
        panic!("{ctx}: {error:?} is neither `warning: ` nor `work: `: {stderr:?}")
    });
    assert!(!message.trim().is_empty(), "{ctx}: empty error message");
    let expected_items = match want {
        Want::List(n) => n,
        _ => 0,
    };
    assert_eq!(
        items.len(),
        expected_items,
        "{ctx}: the `work: ` line must be last, followed only by {expected_items} list items: {stderr:?}"
    );
    for item in items {
        let body = item
            .strip_prefix("  - ")
            .unwrap_or_else(|| panic!("{ctx}: list item {item:?} is not `  - <item>`: {stderr:?}"));
        assert!(
            body.chars().next().is_some_and(|c| !c.is_whitespace()),
            "{ctx}: list item {item:?} must match `^  - \\S`"
        );
    }
}

#[test]
fn the_stderr_shape_rejects_every_malformed_layout() {
    let ok = |stderr: &str, want: Want| {
        std::panic::catch_unwind(|| assert_stderr_shape(stderr, want, "self-test")).is_ok()
    };
    assert!(ok("", Want::Ok));
    assert!(ok("warning: a\nwarning: b\n", Want::Ok));
    assert!(ok("work: boom\n", Want::Error));
    assert!(ok("work: boom\n", Want::Usage));
    assert!(ok("warning: a\nwork: boom\n", Want::Error));
    assert!(ok("warning: a\nwork: boom\n  - x\n  - y\n", Want::List(2)));
    assert!(!ok("progress\n", Want::Ok), "non-warning on success");
    assert!(!ok("work: boom\n", Want::Ok), "error line on success");
    assert!(!ok("", Want::Error), "no error line");
    assert!(!ok("warning: a\n", Want::Error), "warnings only");
    assert!(
        !ok("work: boom\nwarning: late\n", Want::Error),
        "error not last"
    );
    assert!(!ok("work: a\nwork: b\n", Want::Error), "two errors");
    assert!(!ok("other\nwork: boom\n", Want::Error), "stray line first");
    assert!(!ok("work: boom", Want::Error), "no trailing newline");
    assert!(!ok("work: \n", Want::Error), "empty message");
    assert!(!ok("work: boom\n  - x\n", Want::Error), "unexpected items");
    assert!(!ok("work: boom\n  - x\n", Want::List(2)), "too few items");
    assert!(
        !ok("work: boom\n  -  x\n  - y\n", Want::List(2)),
        "bad item"
    );
    assert!(
        !ok("work: boom\n  - x\nwarning: z\n", Want::List(1)),
        "trailing warning"
    );
    let info = |text: &'static str| Want::OkWithStderr(Info::Exact(text));
    assert!(ok("config file exists\n", info("config file exists\n")));
    assert!(!ok("", info("config file exists\n")), "missing info");
    assert!(
        !ok(
            "config file exists\nwork: boom\n",
            info("config file exists\n")
        ),
        "extra error line"
    );
    assert!(
        !ok(
            "warning: a\nconfig file exists\n",
            info("config file exists\n")
        ),
        "extra warning"
    );
    assert!(
        !ok("work: boom\n", Want::OkWithStderr(Info::AboutLogo)),
        "error in logo"
    );
}

/// Rule 3: `sh-*` stdout is one line of script on success, empty otherwise.
fn check_sh_script(out: &Output, args: &[&str], ctx: &str) {
    if !args.first().is_some_and(|cmd| cmd.starts_with("sh-")) {
        return;
    }
    let stdout = text(&out.stdout);
    if out.status.success() {
        assert!(
            stdout.ends_with('\n') && stdout.lines().count() == 1,
            "{ctx}: sh-* must print exactly one line: {stdout:?}"
        );
        assert!(
            stdout.starts_with("cd '"),
            "{ctx}: not a script: {stdout:?}"
        );
    } else {
        assert_eq!(stdout, "", "{ctx}: a failed sh-* must print nothing");
    }
}

/// Rule 4: the sentinel appears nowhere the user or a log could see it.
fn check_no_leak(env: &TestEnv, out: &Output, ctx: &str) {
    let forbidden = forbidden();
    let mut haystacks = vec![
        ("stdout".to_owned(), text(&out.stdout)),
        ("stderr".to_owned(), text(&out.stderr)),
    ];
    for (name, shim) in [
        ("gh", &env.gh),
        ("editor", &env.editor),
        ("opener", &env.opener),
    ] {
        haystacks.push((format!("{name} calls"), format!("{:?}", shim.calls())));
    }
    collect_files(env.root(), &mut haystacks);
    for (name, hay) in &haystacks {
        for needle in &forbidden {
            assert!(
                !hay.contains(needle.as_str()),
                "{ctx}: the token leaked into {name} (as {needle:?})"
            );
        }
    }
}

/// Every regular file under `dir` (not following symlinks, skipping FIFOs
/// and the `gh` shim's own token files, which hold the sentinel on
/// purpose).
fn collect_files(dir: &Path, out: &mut Vec<(String, String)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        let kind = entry.file_type().expect("file type");
        if kind.is_dir() {
            collect_files(&path, out);
        } else if kind.is_file() && !entry.file_name().to_string_lossy().starts_with("gh.token.") {
            let bytes = fs::read(&path).expect("read file");
            out.push((path.display().to_string(), text(&bytes)));
        }
    }
}

fn check_all(cases: &[Case]) {
    for case in cases {
        check(case);
    }
}

/// Every command path, as the argv prefix that reaches it, hidden ones
/// included.
const ALL_COMMANDS: &[&[&str]] = &[
    &["cd"],
    &["on"],
    &["edit"],
    &["clone"],
    &["browse"],
    &["open"],
    &["pull"],
    &["sync"],
    &["list"],
    &["update"],
    &["favorite"],
    &["setup"],
    &["config"],
    &["config", "path"],
    &["config", "default"],
    &["auth"],
    &["auth", "login"],
    &["auth", "logout"],
    &["auth", "status"],
    &["completions"],
    &["activate"],
    &["version"],
    &["about"],
    &["sh-cd"],
    &["sh-on"],
    &["commands"],
];

/// The visible commands whose `--help` is snapshotted.
const VISIBLE_COMMANDS: &[&[&str]] = &[
    &["cd"],
    &["on"],
    &["edit"],
    &["clone"],
    &["browse"],
    &["open"],
    &["pull"],
    &["sync"],
    &["list"],
    &["update"],
    &["favorite"],
    &["setup"],
    &["config"],
    &["config", "path"],
    &["config", "default"],
    &["auth"],
    &["auth", "login"],
    &["auth", "logout"],
    &["auth", "status"],
    &["completions"],
    &["activate"],
    &["version"],
    &["about"],
];

const HIDDEN_COMMANDS: &[&str] = &["sh-cd", "sh-on", "commands"];

/// `config path` when the config file exists.
const CONFIG_EXISTS: Want = Want::OkWithStderr(Info::Exact("config file exists\n"));

/// `completions <shell> --install` (no Homebrew: `PATH` is the shim dir).
const BASH_INSTALLED: Want = Want::OkWithStderr(Info::Exact(
    "installed bash completions to <HOME>/.local/share/bash-completion/completions/work\n",
));
const ZSH_INSTALLED: Want = Want::OkWithStderr(Info::Exact(concat!(
    "installed zsh completions to <HOME>/.local/share/zsh/site-functions/_work\n",
    "add the directory to your fpath before compinit: ",
    "fpath=(<HOME>/.local/share/zsh/site-functions $fpath)\n",
)));
const FISH_INSTALLED: Want = Want::OkWithStderr(Info::Exact(
    "installed fish completions to <HOME>/.config/fish/completions/work.fish\n",
));

#[test]
fn a_bad_flag_is_a_one_line_usage_error_for_every_command() {
    for prefix in ALL_COMMANDS {
        let mut args: Vec<&str> = prefix.to_vec();
        args.push("--no-such-flag");
        let env = build(Fixture::Bare);
        let out = env.command().args(&args).output().expect("run work");
        let ctx = format!("{args:?}");
        check_shape(&env, &out, Want::Usage, &ctx);
        check_sh_script(&out, &args, &ctx);
        check_no_leak(&env, &out, &ctx);
    }
}

#[test]
fn missing_and_invalid_arguments_are_one_line_usage_errors() {
    use Fixture::Bare;
    use Want::Usage;
    check_all(&[
        // A required query is missing.
        case(Bare, &["on"], Usage),
        case(Bare, &["edit"], Usage),
        case(Bare, &["clone"], Usage),
        case(Bare, &["browse"], Usage),
        case(Bare, &["open"], Usage),
        case(Bare, &["pull"], Usage),
        case(Bare, &["sh-on"], Usage),
        // A required subcommand, flag or flag value is missing.
        case(Bare, &["auth"], Usage),
        case(Bare, &["commands"], Usage),
        case(Bare, &["sync", "--jobs"], Usage),
        case(Bare, &["auth", "login", "--domain"], Usage),
        case(Bare, &["auth", "logout", "--domain"], Usage),
        // An argument fails to parse.
        case(Bare, &["sync", "--jobs", "0"], Usage),
        case(Bare, &["sync", "--jobs", "many"], Usage),
        case(Bare, &["activate", "tcsh"], Usage),
        case(Bare, &["completions", "tcsh"], Usage),
        case(Bare, &["favorite", "not a domain"], Usage),
        case(Bare, &["setup", "bad..domain"], Usage),
        case(Bare, &["config", "default", "-bad-"], Usage),
        case(Bare, &["auth", "login", "--domain", "a/b"], Usage),
        // Too many arguments, unknown subcommands, conflicts.
        case(Bare, &["edit", "a", "b"], Usage),
        case(Bare, &["sh-cd", "a", "b"], Usage),
        case(Bare, &["version", "extra"], Usage),
        case(Bare, &["frobnicate"], Usage),
        case(Bare, &["config", "frobnicate"], Usage),
        case(Bare, &["auth", "frobnicate"], Usage),
        case(Bare, &["config", "--edit", "path"], Usage),
        case(Bare, &["list", "--complete", "foo"], Usage),
    ]);
}

const RESOLVERS: &[&str] = &["edit", "clone", "browse", "open", "pull", "sh-cd", "sh-on"];

#[test]
fn successful_runs_never_leak_the_token_and_sh_prints_one_line() {
    use Fixture::{GhToken, Ready};
    use Want::Ok;
    let mut cases = Vec::new();
    for fixture in [Ready, GhToken] {
        cases.extend([
            case(fixture, &["sh-cd", "foo"], Ok),
            case(fixture, &["sh-cd", "bar"], Ok),
            case(fixture, &["sh-cd"], Ok),
            case(fixture, &["sh-on", "acme/foo"], Ok),
            case(fixture, &["sh-on", "bar"], Ok),
            case(fixture, &["edit", "foo"], Ok),
            case(fixture, &["clone", "bar"], Ok),
            case(fixture, &["open", "foo"], Ok),
            case(fixture, &["pull", "foo"], Ok),
            case(fixture, &["sync"], Ok),
            case(fixture, &["list"], Ok),
            case(fixture, &["list", "foo", "--filter"], Ok),
            case(fixture, &["list", "--complete"], Ok),
            case(fixture, &["update"], Ok),
            case(fixture, &["update", "--all-orgs"], Ok),
            case(fixture, &["config"], Ok),
            case(fixture, &["config", "path"], CONFIG_EXISTS),
            case(fixture, &["config", "--edit"], Ok),
            case(
                fixture,
                &["config", "default", "github.com"],
                Want::OkWithStderr(Info::Exact("default domain is now github.com\n")),
            ),
            case(fixture, &["auth", "status"], Ok),
            case(fixture, &["auth", "login"], Ok),
            case(
                fixture,
                &["auth", "logout", "--domain", "ghe.example.com"],
                Ok,
            ),
            case(fixture, &["completions", "bash"], Ok),
            case(
                fixture,
                &["completions", "bash", "--install"],
                BASH_INSTALLED,
            ),
            case(fixture, &["completions", "zsh", "--install"], ZSH_INSTALLED),
            case(
                fixture,
                &["completions", "fish", "--install"],
                FISH_INSTALLED,
            ),
            case(fixture, &["activate", "fish"], Ok),
            case(fixture, &["version"], Ok),
            // Without a terminal, `about` prints a static logo to stderr.
            case(fixture, &["about"], Want::OkWithStderr(Info::AboutLogo)),
            case(
                fixture,
                &["about", "--style", "fire"],
                Want::OkWithStderr(Info::AboutLogo),
            ),
            case(fixture, &["commands", "--sh"], Ok),
            case(fixture, &["--version"], Ok),
            case(fixture, &[], Ok),
        ]);
    }
    // With no config file, `config path` says so.
    cases.push(case(
        Fixture::Bare,
        &["config", "path"],
        Want::OkWithStderr(Info::Exact("config file missing\n")),
    ));
    // A stale cache with the API down warns and still navigates.
    cases.push(case(Fixture::StaleApiDown, &["sh-cd", "foo"], Ok));
    cases.push(case(Fixture::StaleApiDown, &["sh-on", "foo"], Ok));
    cases.push(case(Fixture::NoConfig, &["sh-cd", "foo"], Ok));
    cases.push(case(Fixture::NoConfig, &["list"], Ok));
    check_all(&cases);
}

#[test]
fn browse_launches_the_opener_without_leaking_the_token() {
    for fixture in [Fixture::Ready, Fixture::GhToken] {
        let env = build(fixture);
        let out = env
            .command()
            .args(["browse", "foo"])
            .output()
            .expect("run work");
        check_shape(&env, &out, Want::Ok, "browse foo");
        // The opener is detached: wait for it before scanning its log.
        let calls = env.opener.wait_for_calls(1);
        assert_eq!(calls, [["https://github.com/acme/foo"]]);
        check_no_leak(&env, &out, "browse foo");
    }
}

#[test]
fn commands_that_need_a_terminal_fail_on_one_line() {
    use Fixture::Ready;
    use Want::Error;
    check_all(&[
        case(Ready, &["cd", "foo"], Error),
        case(Ready, &["cd"], Error),
        case(Ready, &["on", "foo"], Error),
        case(Ready, &["favorite"], Error),
        case(Ready, &["setup"], Error),
        case(Ready, &["config", "default"], Error),
        case(Fixture::Ambiguous, &["sh-cd", "foo", "--pick"], Error),
        case(Fixture::Ambiguous, &["sh-on", "foo", "--pick"], Error),
        case(Fixture::Ambiguous, &["edit", "foo", "--pick"], Error),
        // The one list error outside resolving: the valid style names.
        case(Ready, &["about", "--style", "nope"], Want::List(5)),
    ]);
}

#[test]
fn an_invalid_config_is_one_line_for_every_command_that_reads_it() {
    let stage2: &[&[&str]] = &[
        &["cd", "foo"],
        &["on", "foo"],
        &["edit", "foo"],
        &["clone", "foo"],
        &["browse", "foo"],
        &["open", "foo"],
        &["pull", "foo"],
        &["sh-cd", "foo"],
        &["sh-cd"],
        &["sh-on", "foo"],
        &["sync"],
        &["list"],
        &["update"],
        &["favorite"],
        &["setup"],
        &["config"],
        &["config", "default", "github.com"],
        &["auth", "status"],
    ];
    let mut cases = Vec::new();
    for fixture in [Fixture::BadConfig, Fixture::InvalidConfig] {
        cases.extend(stage2.iter().map(|args| case(fixture, args, Want::Error)));
        // Commands that never parse the config still work.
        cases.extend([
            case(fixture, &["config", "path"], CONFIG_EXISTS),
            case(fixture, &["config", "--edit"], Want::Ok),
            case(fixture, &["list", "--complete"], Want::Ok),
            case(fixture, &["version"], Want::Ok),
            case(fixture, &["activate", "bash"], Want::Ok),
        ]);
    }
    check_all(&cases);
}

#[test]
fn no_match_is_one_line_for_every_resolver() {
    let mut cases: Vec<Case> = Vec::new();
    for fixture in [Fixture::Ready, Fixture::NoConfig] {
        for &cmd in RESOLVERS {
            cases.push(case(fixture, &[cmd, NO_MATCH], Want::Error));
        }
    }
    check_all(&cases);
}

#[test]
fn an_ambiguous_query_is_a_list_error_for_every_resolver() {
    let cases: Vec<Case> = RESOLVERS
        .iter()
        .map(|&cmd| case(Fixture::Ambiguous, &[cmd, "foo"], Want::List(2)))
        .collect();
    check_all(&cases);
}

/// Every failure that comes from a cache refresh failing for every domain
/// (a missing or corrupt cache with the API down, `update` with the API
/// down, a missing `gh` with no `GITHUB_TOKEN`), plus `pull` on a corrupt
/// cache, which warns the same way.
fn refresh_failures() -> Vec<Case> {
    let want = Want::Error;
    let mut cases = Vec::new();
    for fixture in [Fixture::NoCache, Fixture::CorruptCache, Fixture::NoGh] {
        for &cmd in RESOLVERS.iter().filter(|cmd| **cmd != "pull") {
            cases.push(case(fixture, &[cmd, "foo"], want));
        }
        cases.extend([
            case(fixture, &["list"], want),
            case(fixture, &["list", "foo"], want),
            case(fixture, &["update"], want),
        ]);
    }
    cases.extend([
        // `pull` never refreshes, but still warns about a corrupt cache.
        case(Fixture::CorruptCache, &["pull", "foo"], want),
        case(Fixture::ApiDown, &["update"], want),
        case(Fixture::ApiDown, &["update", "--all-orgs"], want),
    ]);
    cases
}

#[test]
fn a_failed_refresh_warns_then_ends_in_one_error_line() {
    check_all(&refresh_failures());
}

#[test]
fn a_missing_cache_and_offline_paths_fail_on_one_line() {
    let mut cases = vec![
        // `pull` never refreshes: a missing cache is its own error.
        case(Fixture::NoCache, &["pull", "foo"], Want::Error),
    ];
    for fixture in [Fixture::NoCache, Fixture::CorruptCache] {
        // `list --complete` is offline and silent on failure.
        cases.push(case(fixture, &["list", "--complete"], Want::Ok));
    }
    cases.extend([
        // A fresh cache needs no API, so navigation still works.
        case(Fixture::ApiDown, &["sh-cd", "foo"], Want::Ok),
        case(Fixture::ApiDown, &["list"], Want::Ok),
        case(Fixture::NoGh, &["auth", "login"], Want::Error),
        case(Fixture::NoGh, &["auth", "logout"], Want::Error),
    ]);
    check_all(&cases);
}

#[test]
fn a_broken_stdout_pipe_is_silent_and_exits_0() {
    let env = build(Fixture::Ready);
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin!("work"))
        .env_clear()
        .envs(env.vars())
        .current_dir(env.home())
        .arg("list")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn work");
    // Close the only read end before `work` writes anything.
    drop(child.stdout.take());
    let out = child.wait_with_output().expect("wait");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(text(&out.stderr), "");
}

#[test]
fn every_shell_integrated_command_has_an_sh_subcommand() {
    let env = build(Fixture::Bare);
    let out = env
        .command()
        .args(["commands", "--sh"])
        .output()
        .expect("run work");
    assert!(out.status.success());
    let names = text(&out.stdout);
    assert_ne!(names, "");
    for name in names.lines() {
        env.command()
            .args([format!("sh-{name}").as_str(), "--help"])
            .assert()
            .success();
    }
}

fn help(args: &[&str]) -> String {
    let env = build(Fixture::Bare);
    let out = env
        .command()
        .args(args)
        .arg("--help")
        .output()
        .expect("run work");
    assert!(out.status.success(), "{args:?} --help");
    assert_eq!(text(&out.stderr), "", "{args:?} --help writes only stdout");
    text(&out.stdout)
}

#[test]
fn top_level_help_is_snapshotted_and_hides_internal_commands() {
    let top = help(&[]);
    for hidden in HIDDEN_COMMANDS {
        assert!(
            !top.lines()
                .any(|line| line.trim_start().starts_with(&format!("{hidden} "))),
            "hidden command {hidden} is listed:\n{top}"
        );
    }
    for visible in VISIBLE_COMMANDS.iter().filter(|cmd| cmd.len() == 1) {
        assert!(
            top.lines()
                .any(|line| line.trim_start().starts_with(&format!("{} ", visible[0]))),
            "{} is missing:\n{top}",
            visible[0]
        );
    }
    insta::assert_snapshot!("help_work", top);
}

#[test]
fn every_visible_subcommand_help_is_snapshotted() {
    for args in VISIBLE_COMMANDS {
        let name = format!("help_{}", args.join("_"));
        insta::assert_snapshot!(name, help(args));
    }
}

#[test]
fn hidden_flags_stay_out_of_help() {
    assert!(!help(&["list"]).contains("--complete"));
    assert!(!help(&["about"]).contains("--style"));
}

#[test]
fn the_leak_scan_finds_every_encoding_of_the_sentinel() {
    for needle in forbidden() {
        let env = build(Fixture::Bare);
        fs::write(env.home().join("planted.txt"), &needle).expect("plant");
        let out = env.command().arg("version").output().expect("run work");
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            check_no_leak(&env, &out, "planted");
        }));
        assert!(caught.is_err(), "{needle} was not detected");
    }
    assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    assert_eq!(base64(b"fooba"), "Zm9vYmE=");
    assert_eq!(base64(b"foob"), "Zm9vYg==");
}

/// The subcommands listed under `Commands:` in `work <prefix> --help`,
/// without clap's own `help`.
fn listed_subcommands(prefix: &[&str]) -> Vec<String> {
    help(prefix)
        .lines()
        .skip_while(|line| *line != "Commands:")
        .skip(1)
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| *name != "help")
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_command_tables_match_the_commands_help_lists() {
    let mut walked: Vec<Vec<String>> = Vec::new();
    let mut queue: Vec<Vec<String>> = vec![Vec::new()];
    while let Some(prefix) = queue.pop() {
        let args: Vec<&str> = prefix.iter().map(String::as_str).collect();
        for name in listed_subcommands(&args) {
            let mut path = prefix.clone();
            path.push(name);
            walked.push(path.clone());
            queue.push(path);
        }
    }
    let mut walked: Vec<String> = walked.iter().map(|path| path.join(" ")).collect();
    walked.sort();
    let mut visible: Vec<String> = VISIBLE_COMMANDS.iter().map(|path| path.join(" ")).collect();
    visible.sort();
    assert_eq!(walked, visible, "VISIBLE_COMMANDS is out of date");
    let mut all: Vec<String> = ALL_COMMANDS.iter().map(|path| path.join(" ")).collect();
    all.sort();
    let mut expected = visible;
    expected.extend(HIDDEN_COMMANDS.iter().map(|name| (*name).to_owned()));
    expected.sort();
    assert_eq!(
        all, expected,
        "ALL_COMMANDS must be the visible plus the hidden commands"
    );
}
