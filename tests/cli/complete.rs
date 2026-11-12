//! Tab completion: the hidden `sh-complete` the completion scripts call.

use crate::support::{TestEnv, text};

/// A home with two domains, repos in two orgs, and `Zebra` used often.
fn home() -> TestEnv {
    let env = TestEnv::new();
    env.config_domains(&["github.com", "ghe.example.com"]);
    env.seed_cache(
        &[
            "github.com/acme/app",
            "github.com/acme/Zebra",
            "github.com/other/app",
            "ghe.example.com/corp/api",
        ],
        false,
    );
    env.seed_frecency(&[("github.com/acme/Zebra", 9)]);
    env
}

/// The candidates `shell` is given for `words` (the last is under the cursor).
fn candidates(env: &TestEnv, shell: &str, words: &[&str]) -> Vec<String> {
    let out = env
        .command()
        .args(["sh-complete", shell, "--"])
        .args(words)
        .output()
        .expect("run");
    assert!(out.status.success());
    assert_eq!(text(&out.stderr), "");
    text(&out.stdout).lines().map(str::to_owned).collect()
}

#[test]
fn repos_complete_by_name_most_used_first() {
    let env = home();
    assert_eq!(
        candidates(&env, "bash", &["cd", ""]),
        ["Zebra", "api", "app"]
    );
    assert_eq!(candidates(&env, "bash", &["edit", "z"]), ["Zebra"]);
    // The first word is taken: a repo command resolves one repo.
    assert_eq!(candidates(&env, "bash", &["cd", "app", ""]), [""; 0]);
}

#[test]
fn each_slash_completes_a_longer_form_of_the_repo() {
    let env = home();
    assert_eq!(
        candidates(&env, "bash", &["browse", "a"]),
        ["acme/Zebra", "acme/app", "api", "app"]
    );
    assert_eq!(
        candidates(&env, "bash", &["browse", "other/"]),
        ["other/app"]
    );
    assert_eq!(
        candidates(&env, "bash", &["browse", "ghe.example.com/"]),
        ["ghe.example.com/corp/api"]
    );
}

#[test]
fn subcommands_and_flags_complete_without_the_hidden_ones() {
    let env = TestEnv::new();
    let top = candidates(&env, "bash", &[""]);
    assert!(top.contains(&"browse".to_owned()), "{top:?}");
    assert!(top.iter().all(|c| !c.starts_with("sh-")), "{top:?}");
    assert_eq!(candidates(&env, "bash", &["config", "d"]), ["default"]);
    assert_eq!(
        candidates(&env, "bash", &["help", "auth", "log"]),
        ["login", "logout"]
    );

    assert_eq!(
        candidates(&env, "bash", &["list", "--"]),
        ["--filter", "--help"]
    );
    assert_eq!(
        candidates(&env, "bash", &["cd", "--pick", "--"]),
        ["--help"]
    );
    assert_eq!(
        candidates(&env, "bash", &["completions", ""]),
        ["bash", "zsh", "fish"]
    );
}

#[test]
fn domains_complete_from_the_config() {
    let env = home();
    let domains = ["ghe.example.com", "github.com"];
    assert_eq!(candidates(&env, "bash", &["favorite", ""]), domains);
    assert_eq!(
        candidates(&env, "bash", &["config", "default", "gi"]),
        ["github.com"]
    );
    assert_eq!(
        candidates(&env, "bash", &["auth", "login", "--domain", ""]),
        domains
    );
    // Bash splits `--domain=gi` at the `=` and replaces only the last word.
    assert_eq!(
        candidates(&env, "bash", &["auth", "login", "--domain", "=", "gi"]),
        ["github.com"]
    );
    assert_eq!(
        candidates(&env, "zsh", &["auth", "login", "--domain=gi"]),
        ["--domain=github.com"]
    );
    // `--jobs` takes a number: nothing to offer, least of all a repo.
    assert_eq!(candidates(&env, "bash", &["sync", "--jobs", ""]), [""; 0]);
}

#[test]
fn zsh_and_fish_get_descriptions() {
    let env = TestEnv::new();
    assert_eq!(
        candidates(&env, "fish", &["upd"]),
        ["update\tRefresh the repository cache"]
    );
    assert_eq!(
        candidates(&env, "zsh", &["upd"]),
        ["update:Refresh the repository cache"]
    );
}

#[test]
fn completing_never_touches_the_network_or_complains() {
    // No config and no cache.
    let env = TestEnv::new();
    assert_eq!(candidates(&env, "bash", &["cd", ""]), [""; 0]);

    // A stale cache is used as it is, not refreshed.
    env.config_domains(&["github.com"]);
    env.seed_cache(&["github.com/acme/old"], true);
    assert_eq!(candidates(&env, "bash", &["cd", ""]), ["old"]);

    // A config that does not parse.
    env.config("work_dir = [");
    assert_eq!(candidates(&env, "bash", &["cd", ""]), [""; 0]);
    assert_eq!(env.calls("gh"), [""; 0]);
}
