# Sync Operations

Sync operations keep locally-cloned repositories up to date by fetching from remotes and fast-forwarding when safe. They are conservative — they never force-push, rebase, or overwrite local changes.

## Commands

### work pull \<query\>

Sync a single repository.

- Resolves the query to a specific repository
- Fetches from origin
- Compares against the branch's configured upstream (`@{u}`), never an assumed `origin/<branch>`, so a branch tracking a differently named remote branch syncs correctly and a branch with no upstream is reported as fetched-only
- If the worktree is clean (no dirty files, no staged changes, no untracked files):
  - Checks if the current branch can be fast-forwarded to its upstream
  - If yes, fast-forwards and reports how many commits were pulled
  - If no (diverged), reports that the repo was fetched only
- If the worktree is dirty: only fetches, reports "fetched (dirty worktree)"
- Does NOT trigger a cache auto-update (it is a local git operation)

### work sync

Sync ALL locally-cloned repositories.

- Syncs every repository that is both in the cache and cloned in the work directory (its expected path contains `.git`). Directories under the work directory that are not in the cache are ignored, as are cached repos that aren't cloned.
- Requires the cache. With no cache, it fails and suggests `work update`.
- Performs the same fetch + conditional fast-forward on each
- Reports status for each repository:
  - "pulled (N commits)" — fast-forwarded successfully ("pulled (1 commit)" when N is 1)
  - "up to date" — already at the latest commit
  - "fetched (\<reason\>)" — fetched but did not fast-forward; the reason is one of "dirty worktree", "diverged", "detached HEAD", or "no upstream"
  - "error: \<message\>" — something went wrong
- Does NOT trigger a cache auto-update

## Sync Result States

For each repository, the outcome is one of:

1. **Pulled** — Clean worktree, fast-forward succeeded. Reports commit count.
2. **Up to date** — Clean worktree, already at upstream. Nothing to do.
3. **Fetched** — Fetch completed but no merge attempted, with a reason: dirty worktree, diverged, detached HEAD, or no upstream configured (a local-only branch). None of these is an error; the repo simply has nothing safe to fast-forward.
4. **Error** — Something failed (not a git repo, network error, etc.)

## Safety Guarantees

- Never modifies files in a dirty worktree
- Never performs a merge (only fast-forward)
- Never force-updates branches
- Never touches branches other than the current one
- If fast-forward is not possible (local commits exist that are not on remote), falls back to fetch-only

## Authentication

Sync operations use the same auth token resolution as other operations. Tokens are passed to git via HTTP authorization headers for fetching from private repositories.

- `pull`: resolves the token from the repo's domain (known from the cache).
- `sync`: resolves each repo's token from its domain (known from the cache), the same as `pull`.
- The token is scoped to `https://<domain>`, so git never sends it to another host, even if `origin` points elsewhere. If no token is available, the fetch proceeds without authentication (may still work for public repos, ssh remotes, or when git has stored credentials).
