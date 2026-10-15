# Authentication

The GitHub CLI (`gh`) owns authentication. `work` never reads, stores, or forwards a token.

## Prerequisites

- `gh` is installed and available in `PATH`.
- The user is logged in to each GitHub domain they use, through `gh auth login` or `work auth login`.

## How Credentials Are Used

- **API calls.** Every GitHub API request is a `gh api --hostname <domain>` call. `gh` picks the credential for that host.
- **git.** Clones and fetches use `gh auth git-credential` as git's credential helper, scoped to the repository's domain. git asks `gh` for a credential only for that host.
- **Environment tokens.** `GH_TOKEN`, `GITHUB_TOKEN`, `GH_ENTERPRISE_TOKEN` and `GITHUB_ENTERPRISE_TOKEN` work as `gh` defines them. A github.com token is never sent to an Enterprise host.

## Auth Commands

### `work auth login [--domain <domain>]`

Runs `gh auth login --hostname <domain>` with the terminal attached, so `gh` handles the interactive flow. The domain defaults to `github.com`.

### `work auth logout [--domain <domain>]`

Runs `gh auth logout --hostname <domain>` with the terminal attached. The domain defaults to `github.com`.

### `work auth status`

Runs `gh auth status --hostname <domain>` for every configured domain and shows its output. Exits non-zero if any domain is not logged in.

## Error Behavior

- **Not logged in, or missing access.** The operation fails with `gh`'s own message, reduced to one line. It names the domain and how to fix it.
- **`gh` not installed.** The error says the GitHub CLI is required and must be installed.
