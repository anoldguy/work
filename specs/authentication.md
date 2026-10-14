# Authentication

The `work` CLI requires GitHub API tokens to interact with repository data. It does not manage credentials directly — it delegates all credential storage to the GitHub CLI (`gh`).

## Prerequisites

- The GitHub CLI (`gh`) must be installed and available in `PATH`.
- The user must have run `gh auth login` for each GitHub domain they want to use, or we will run it for them, wrapping the invocation.

## Token Resolution

When a token is needed for a given domain, it is resolved in this order:

1. **`GITHUB_TOKEN` environment variable** — If set and non-empty, this token is used for all domains unconditionally.
2. **GitHub CLI** — The tool runs `gh auth token --hostname <domain>` to retrieve the token for the specific domain.

The first source that produces a valid token wins. If neither source provides a token, the operation fails.

## Session Caching

Once a token is successfully resolved for a domain, the result is cached in memory for the lifetime of the process. Subsequent operations against the same domain reuse the cached token without invoking `gh` again.

## Auth Commands

### `work auth login [--domain <domain>]`

Shells out to `gh auth login --hostname <domain>`, letting `gh` handle the interactive authentication flow. Domain defaults to `github.com`.

### `work auth logout [--domain <domain>]`

Shells out to `gh auth logout --hostname <domain>`, letting `gh` handle the logout. Domain defaults to `github.com`.

### `work auth status`

Displays authentication status for every configured domain. For each domain, reports:

- Whether a valid token is available.
- The token source (`GITHUB_TOKEN` env var or `gh` CLI).

## Error Behavior

- **No token available** — Operations requiring the GitHub API (e.g., cache updates) fail with a clear message directing the user to run `gh auth login --hostname <domain>`.
- **`gh` not installed** — The error message indicates that the GitHub CLI is required and must be installed.
