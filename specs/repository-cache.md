# Repository Cache

## Purpose

The tool maintains a local cache of all repositories the user has access to across all configured GitHub domains. This enables fast search, tab completion, and navigation without hitting the GitHub API on every command.

## Cache Contents

The cache stores:

- A list of repositories, each with: slug (`domain/org/repo`), clone URL, and domain.
- A timestamp of when the cache was last updated.
- Frecency data for each repository (access count and last access time) to personalize search ranking.

## Cache Location

Stored as a JSON file in the configured cache directory (default: `~/.local/state/work/cache.json`). The cache directory and parent directories are created automatically if they don't exist.

## Staleness

- The cache is considered stale if it was last updated more than 24 hours ago.
- When a command that needs the cache runs and the cache is stale, an auto-update is triggered before the command executes.
- The `sync` and `pull` commands skip auto-update (they are local git operations).
- If no cache file exists at all, an update is triggered automatically.

## Cache Update

`work update [--all-orgs]`

Running `work update` always performs a full cache refresh regardless of staleness. If the user ran it manually, they want an update.

During a cache update:

1. For each configured domain, the tool fetches:
   - All personal repositories: those the user owns and those they're a collaborator on (owned by other users). Global org filters apply to these by owner; favorites do not.
   - All repositories from organizations the user belongs to.
2. Organization filtering:
   - Global `include_orgs` / `exclude_orgs` filters are applied first.
   - Then per-domain `favorite_orgs` are applied (unless `--all-orgs` is passed).
   - If a domain has favorites configured and `--all-orgs` is NOT set, only favorite orgs are synced.
   - If no favorites are configured, all orgs (after global filters) are synced.
3. Organization repos are fetched concurrently (up to 8 orgs at a time).
4. Progress is shown via spinner indicators during the update.
5. The `--all-orgs` flag ignores per-domain favorites for this update.

## Frecency Preservation

When the cache is updated, frecency data (access counts and timestamps) from the previous cache is preserved for repositories that still exist. Frecency entries for repositories that no longer appear in the update are cleaned up.

## Cache Creation

If the cache doesn't exist when a command needs it, a full update is performed automatically. The spinner writes to stderr, so it does not interfere with stdout-based shell eval output from `sh-` commands.

## Error Behavior

Failures are contained, reported, and never destructive. A refresh replaces the whole repo list, so anything that fails to fetch keeps its previous cache entries rather than vanishing (which would also garbage-collect its frecency history).

- If an individual org fetch fails, a warning naming the org and cause is printed to stderr, other orgs continue, and the org's previously cached repos are carried forward.
- If a domain cannot be refreshed (no token, network failure, org listing failed), a warning is printed, other domains continue, and the domain's previously cached repos are carried forward.
- If every configured domain fails, the update returns an error and nothing is written, so the existing cache and its timestamp are kept and the next command retries.
- Warnings are printed whether or not the progress UI is shown. In the interactive UI they are held until the spinners finish so they are not painted over.
- If the cache file can't be read (corrupted, permissions), an update is triggered to recreate it.
