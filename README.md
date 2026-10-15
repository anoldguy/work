# work

A CLI tool that organizes repositories by organization and name in `~/code/{domain}/{org}/{repo}`. Jump to code faster; spend less time wondering where you cloned that thing three months ago.

No database, no daemon, no cloud sync that stops working when your wifi hiccups.

## Should You Use This?

**Good fit:**
- You work across multiple GitHub orgs and/or a GitHub Enterprise instance
- You've typed `find ~ -name '.git' -type d` more than once this month
- You want `cd` but for repos, with fuzzy matching and the memory of a goldfish (frecency)
- You're comfortable with CLI tools and don't need a GUI to feel safe

**Not a good fit:**
- You have one repo. Congratulations on your focus
- You already have a system that works (don't fix what isn't broken, I'm not your mom)
- You want something battle-tested by thousands of users (this is a hobby project, there are dozens of us)

## Install

The easy way is the shell installer from the latest release:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/anoldguy/work/releases/latest/download/work-installer.sh | sh
```

Prebuilt binaries exist for Linux and macOS, on x86_64 and aarch64. Prefer to build it yourself:

```bash
cargo install --git https://github.com/anoldguy/work
# or, from a checkout:
cargo build --release   # binary at target/release/work
```

Make sure the binary is on your `PATH` before moving on to shell integration; `work activate` can't wrap what it can't find.

## Prerequisites

- [GitHub CLI (`gh`)](https://cli.github.com/) (authentication -- implementing OAuth device flow just to avoid typing `gh auth login` seemed like the wrong kind of yak shaving)
- Git 2.12 or newer (for `status --porcelain=v2` and `protocol.<name>.allow`)
- Linux or macOS. Windows is not supported, and nobody is pretending otherwise

### Shell Integration

A program can't change its parent shell's directory, so `cd` and `on` need a small shell function wrapped around the binary. Without it, `work cd` exits with an error pointing you at `work activate`, rather than printing a path and leaving you to do the walking. Add to your shell config:

```bash
# bash (~/.bashrc)
eval "$(work activate bash)"

# zsh (~/.zshrc)
eval "$(work activate zsh)"

# fish (~/.config/fish/config.fish)
work activate fish | source
```

Restart your shell or source the file. Run `type work` to verify; it should show a function, not just a path.

### Tab Completion

```bash
work completions bash --install
work completions zsh --install
work completions fish --install
```

For zsh, `--install` also prints an `fpath` hint on stderr, because a child process can't see your `fpath`; follow it. Tab completion turns `work cd my<TAB>` into something useful. Without it you're just typing full repo names like an animal.

## Quick Start

Three steps: authenticate, enable shell integration, navigate to repositories. That's it.

```bash
# First-time setup -- walks you through config and auth
work setup

# Jump to a repo (fuzzy match, auto-clones if needed)
work cd myrepo

# Open a repo in your editor
work edit myrepo

# Open a repo on GitHub in the browser
work browse myrepo
```

Repository lists are cached locally and refresh automatically when stale. Hitting the GitHub API on every typo would be excessive, and typos happen more than anyone wants to admit. You never need to think about the cache -- it refreshes itself (under a spinner, in the foreground; there is no daemon) the first time a command notices it's stale. If you're impatient, `work update` forces a refresh, but you probably won't need it.

## Search & Frecency

Search ranks repositories by match quality and usage patterns. The repo you fat-finger every morning beats the exact match you haven't touched since Q2. This is called frecency scoring (frequency + recency) and it works better than pretending all matches are equally important.

Every time you interact with a repo (`cd`, `browse`, `clone`, `edit`, `on`, `open`, `pull`), it records the access. Recent activity counts more than old activity: last week gets full weight, last month gets half, last quarter gets 20%, and anything older than 90 days barely registers. The repo you mistype as `widgts` constantly still beats `widgets-archive` which you spelled perfectly but cloned once in 2023.

All matching is case-insensitive. Type `FOO` or `foo` or `FoO`; same result. Life's too short for shift keys.

For more specificity, qualify with org (`acme/foo`) or full slug (`github.com/acme/foo`). Each slash narrows the search. Multiple matches show an interactive picker unless you're in a script, where ambiguity errors instead of picking randomly and hoping. Pass `--pick` to `cd`, `on`, `edit`, `open`, `browse`, `clone` or `pull` to see the picker whenever several repositories match, even if one is clearly ahead.

### The Picker

Type to filter, Backspace to edit what you typed. Arrow keys or Ctrl-n/Ctrl-p move the selection, Enter selects, Esc cancels. `j` and `k` are not navigation keys; they are letters, and you may need them. In multi-select pickers (`work favorite`), Tab toggles an item. Ctrl-C bails out with exit code 130.

## Commands

| Command | What it does |
|---------|-------------|
| `work version` | Print the version |
| `work cd [query]` | Jump to a repository (needs shell integration; clones first if needed; omit the query to go to the work directory) |
| `work on <query>` | Jump to a repository and open it in your editor (needs shell integration), for the tab hoarders |
| `work edit <query>` | Open a repository in your editor (`$WORK_EDITOR`, then `$EDITOR`, then `vi`) |
| `work open <query>` | Open a repository's directory in the file manager (clones first if needed; for the GUI-curious) |
| `work browse <query>` | Open a repository's page in the browser |
| `work clone <query>` | Clone a repository into the work directory |
| `work list [query]` | List cached repositories, ranked by relevance (`--filter` for a plain substring match on the full slug, without frecency) |
| `work pull <query>` | Fetch and fast-forward one repository |
| `work sync [--jobs <n>]` | Fetch and fast-forward every cached repository you have cloned (8 at a time by default). Go get coffee. Exits 1 if any repo failed |
| `work update [--all-orgs]` | Refresh the repository cache (usually unnecessary; `--all-orgs` fetches every org, ignoring favourites) |
| `work config` | Show or change configuration (bare, it shows the current one) |
| `work config path` | Show the config file location and whether it exists (path on stdout, existence on stderr) |
| `work config --edit` | Open the config file in your editor (`$WORK_EDITOR`, then `$EDITOR`, then `vi`) |
| `work config default [domain]` | Set the default domain (asks when omitted) |
| `work favorite [domain]` | Pick favourite organisations for a domain (only cache repos from orgs you care about) |
| `work setup [domain]` | Interactive first-time setup (default domain: `github.com`) |
| `work auth login [--domain <domain>]` | Log in with `gh` (default domain: `github.com`) |
| `work auth logout [--domain <domain>]` | Log out with `gh` (default domain: `github.com`) |
| `work auth status` | Show token status for every domain |
| `work completions [shell] [--install]` | Print or install shell completions |
| `work activate [shell]` | Print the shell integration script (auto-detects from `$SHELL` if omitted) |
| `work about` | Show the animated about screen; any key exits. Styles: `classic`, `decode`, `plasma`, `fire`, `dropin` (via a hidden `--style <name>` flag). Without a TTY it prints the static logo to stderr |

## Configuration

Configuration is optional. Defaults work for most people: `~/code` for repositories, `~/.local/state/work` for cache. XDG conventions are followed; somebody should.

Config lives at `~/.config/work/config.toml`. `work setup` creates it; until then, the defaults apply, with an implicit `github.com` as the default domain. Edit it by hand if you prefer (`work config --edit` opens it in `$WORK_EDITOR`, then `$EDITOR`, then `vi`); it's just TOML. `work config path` prints the bare path on stdout and says whether the file exists on stderr, so `cat "$(work config path)"` behaves.

```toml
work_dir = "~/code"
cache_path = "~/.local/state/work"

[filters]
exclude_orgs = ["that-org-from-2019-you-forgot-to-leave"]
include_orgs = ["my-team"]

[domains."github.com"]
is_default = true
favorite_orgs = ["my-team", "my-company"]

[domains."github.enterprise.com"]
favorite_orgs = ["team-a", "team-b"]
extra_orgs = ["partner-team", "shared-infra"]
api_url = "https://github.enterprise.com/api/v3"
```

Per domain, `api_url` overrides the API base. You rarely need it. The default is `api.github.com` for `github.com`, `https://api.<host>` for `*.ghe.com`, and `https://<host>/api/v3` for GitHub Enterprise Server. It has to be `https`, or `http` for loopback hosts only, because sending your token over plain HTTP to a stranger is not a feature. Unknown keys are rejected with an error rather than silently ignored, so a typo like `favourite_orgs` tells you instead of quietly doing nothing. Quote the hostname (`[domains."github.com"]`); an unquoted one gets a hint.

## Default Domains

One domain can be marked as default to shorten filesystem paths. The default domain's repositories live at `~/code/org/repo`. Other domains keep the full path: `~/code/domain/org/repo`.

The first domain you configure gets marked as default automatically. Change the default anytime:

```bash
work config default                   # Interactive picker
work config default github.com        # Set specific domain
```

Changing the default doesn't move existing repositories. You'll need to re-clone or manually relocate them to match the new paths.

## Favorite Organizations

Cache updates sync all organizations you belong to, plus any extra orgs you've configured. If you just got added to 47 orgs at your new enterprise job and only care about 3 of them, this gets tedious.

Configure favorite organizations to sync only those:

```bash
work favorite                        # Interactive picker for default domain
work favorite github.enterprise.com  # Pick favorites for specific domain
```

Once configured, only favorite orgs (plus personal repositories) get synced. Personal repositories include the ones you collaborate on, not just the ones you own. Use `work update --all-orgs` to sync everything, ignoring favorites. Clear favorites by deselecting everything in the picker; you're back to syncing all organizations.

## Extra Organizations

GitHub's API only returns organizations you're a member of. This is fine until you're working with a GitHub Enterprise instance where you have access to repos in an org through team or collaborator permissions without being a formal member. The repos exist, you can clone them, but the tool doesn't know they're there because the API never mentions them.

The `extra_orgs` setting fixes this. Add org names to a domain's config and they get included in every cache update alongside your member orgs:

```toml
[domains."github.enterprise.com"]
extra_orgs = ["partner-team", "shared-infra"]
favorite_orgs = ["my-team", "partner-team"]
```

Extra orgs flow through the same filtering pipeline as everything else. Global include/exclude filters apply. Favorites narrow the list if configured. If you later become a formal member of an extra org, the tool deduplicates automatically; nothing breaks, nothing doubles.

Extra orgs also appear in the `work favorite` picker, so you can favorite them without editing the config file by hand.

## Authentication

The tool uses GitHub CLI. No tokens are stored directly. GitHub CLI already solved this problem; reimplementing it felt like trust issues with extra steps.

```bash
# Standard GitHub
gh auth login

# GitHub Enterprise
gh auth login --host github.enterprise.com
```

Run `gh auth login` once; everything else works.

If `GITHUB_TOKEN` is set, it is used for every domain and `gh` is not consulted. That includes your GitHub Enterprise host, so a token exported for some other tool will be sent there, and an invalid one is a hard error with no fallback to `gh`. Unset it to use your per-host `gh` logins. Check `work auth status` if things seem broken.

## How It Works

`work` maintains a local JSON cache of repo metadata fetched from the GitHub REST API. `work` makes those HTTPS calls itself; `gh` only supplies the token. When you run `work cd foo`, it:

1. Fuzzy-matches "foo" against all cached repos
2. Ranks results by frecency (repos you use often float to the top)
3. If there's one clear winner, `cd`s you there
4. If it's ambiguous and you're in an interactive terminal, shows a picker
5. If the repo isn't cloned locally, clones it first

The cache auto-updates when stale (older than 24 hours). This happens transparently during normal use -- you don't need to manually refresh anything. The tool fetches new data while a spinner pretends to keep you company.

### Offline

If the cache is stale and GitHub is unreachable, each command waits for the fetch timeout once, then falls back to the stale copy with a warning on stderr. It's slower than it should be and still works, which describes most airline wifi.

The local directory structure mirrors GitHub's hierarchy:

```
~/code/
  acme/              # default domain omits the prefix
    api/
    web/
  github.enterprise.com/
    corp-team/
      internal-tool/
```

## Troubleshooting

**"repository not found: <query>"**
Stale cache. Run `work update` to force a refresh. This happens when you join new organizations or when your coworker creates another microservice at 4pm on Friday.

**Shell integration not working**
The activation line didn't load. Run `type work` to verify; it should show a function, not a path. Check your shell config file has the activation line. Source it. Yes, I forget this step too.

**Authentication failures**
Check `work auth status` first. Usually the fix is `gh auth login`. Sometimes it's realizing you're on the VPN but pointing at the wrong GitHub instance.

**Cloning fails**
This tool shells out to `git clone`. If git can't clone it manually, neither can the tool. Your SSH keys, HTTPS credentials, or repository access might be the problem.

## License

[MIT](LICENSE)
