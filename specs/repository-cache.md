# Repository Cache

## Purpose

The repository cache is a local list of every repository the user can reach across all configured GitHub domains. It makes search, tab completion, and navigation fast without calling the GitHub API on every command.

## Cache Contents

The cache stores:

- A list of repositories, each with its slug (`domain/org/repo`), clone URL, and web URL.
- A timestamp of when the list was last updated.

The cache holds no usage history. Frecency lives in its own file (see [search-resolution.md](search-resolution.md#frecency-storage)), and nothing in this spec reads or changes it.

## Cache Location

`{cache_path}/cache.json` (default: `~/.local/state/work/cache.json`). The directory and its parents are created when needed.

## Staleness

- The cache is stale when it was last updated more than 24 hours ago, or when its timestamp is more than 5 minutes in the future.
- A command that needs the cache refreshes it first when it is missing, unreadable, or stale. The command waits for the refresh, which shows live progress on stderr.
- `sync` and `pull` never refresh the cache. They are local git operations.

## Cache Update

`work update [--all-orgs]`

`work update` always refreshes the cache, however fresh it is.

During a refresh, for each configured domain:

1. `gh` lists the user's personal repositories: those they own and those they collaborate on (owned by other users). Global org filters apply to these by owner; favorites do not.
2. `gh` lists the repositories of the organizations the user belongs to, plus the domain's `extra_orgs`.
3. Organization filtering:
   - Global `include_orgs` / `exclude_orgs` filters apply first.
   - Then per-domain `favorite_orgs` apply, unless `--all-orgs` is passed.
   - If a domain has favorites and `--all-orgs` is not set, only favorite orgs are listed.
   - If a domain has no favorites, every org that passes the global filters is listed.
4. Listings run concurrently, up to 8 at a time across all domains.
5. Progress shows on stderr: a header with the overall count, then one spinner line per listing in flight.
6. `--all-orgs` ignores per-domain favorites for this update only.

A repository that appears in more than one listing is stored once.

After the refresh, `work update` re-checks adopted repositories (see [Adopted Repositories](#adopted-repositories)). An update that fails for every domain re-checks nothing.

## Error Behavior

Failures are contained and reported. A refresh replaces the whole repository list, so anything that fails to list keeps its previous entries.

- If one listing fails, a warning naming it and the cause goes to stderr, the other listings continue, and that org's previously cached repos stay in the cache.
- If a domain cannot be refreshed (not logged in, network failure, the org listing failed), a warning goes to stderr, other domains continue, and the domain's previously cached repos stay.
- If every configured domain fails, the update returns an error and writes nothing. The existing cache and its timestamp stay, and the next command tries again. A command that only needed a refresh because the cache was stale warns and uses the stale cache instead.
- Entries that fail validation (a malformed name or a URL that is not on the domain) are skipped, with one warning per listing giving the count.
- Warnings appear whether or not progress is shown. With live progress, they are held until the progress display is cleared.
- An unreadable cache file (corrupt, wrong version, permissions) is renamed to `cache.json.corrupt` once a refresh succeeds, and the new cache replaces it.

## Adopted Repositories

Listings cover the repositories the user owns or collaborates on, and those of member organizations and `extra_orgs`. Any other repository the user can read, such as a public repository in another organization or an internal repository on an enterprise server, becomes known when resolution finds it on GitHub (see [search-resolution.md](search-resolution.md#lookup-on-github)). Such a repository is *adopted*.

- Adopted repositories live in `{cache_path}/adopted.json`, separate from `cache.json`. Each entry stores the slug, clone URL and web URL.
- Search, tab completion, `list`, `sync`, `pull`, `browse` and frecency treat adopted repositories exactly like cached ones. A repository that is both listed and adopted appears once (slugs compare ignoring ASCII case).
- An adopted repository on a domain that is not configured is ignored by everything except `work forget`, and stays in the file.
- Refreshing the cache never reads or changes `adopted.json`.
- Each write reloads the file under a lock, applies the change, and replaces the file atomically, so concurrent `work` processes never lose each other's adoptions.
- A file with an unknown version is never overwritten. Adoption is read-only for that run (the repository still resolves), and one warning says so.
- An unparsable file is renamed to `adopted.json.corrupt` with a warning, and the list starts empty.

### Re-checking

`work update` looks up every adopted repository on a configured domain again, with the same `gh api` call as resolution, up to 8 at a time:

- A repository GitHub answers "not found" (HTTP 404) is dropped, with one warning naming it.
- A repository GitHub answers under another owner or name is stored under its current slug.
- Any other failure (network, server error, refused access, an invalid answer) keeps the repository unchanged.

### work forget \<query\>

Removes one adopted repository.

- The query resolves against adopted repositories only, as described in [search-resolution.md](search-resolution.md#resolution), including `--pick`, the selector and the ambiguity error. Nothing is looked up on GitHub.
- On success, prints `Forgot <slug>` to stdout. The clone and frecency are left alone.
- With no adopted match: error, `no adopted repository matches: <query>`.
- A forgotten repository that is still listed stays known. Resolving it on GitHub adopts it again.
