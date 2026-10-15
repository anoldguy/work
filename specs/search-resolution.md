# Search and Repository Resolution

## Repo Queries

Most commands accept a "repo query": a partial or full name that identifies a repository. Queries come in three levels of specificity:

- **Repo name only** (`foo`) matches against the repository name (the last segment of the slug).
- **Org/repo** (`acme/foo`) matches against the org and repo name (the last two segments).
- **Full slug** (`github.com/acme/foo`) matches the entire slug.

Matching is case-insensitive at every level.

## Search Algorithm

Search combines match quality with usage frequency.

### Match Quality Tiers

1. **Exact match (score: 100).** The query exactly matches at its level of specificity. `foo` exactly matches a repo named `foo`, but not `foobar`.
2. **Substring match (score: 50).** The query appears as a substring of the repo name or the full slug.
3. **Fuzzy match (score: 1–49).** The query matches as a fuzzy subsequence. Scores are scaled relative to the query matched against itself, so they stay below the substring tier.
4. **No match.** Repos that match at no tier are excluded.

### Frecency Scoring

Each repository has a frecency score based on how the user uses it:

- Every successful `cd`, `on`, `edit`, `clone`, `browse`, `open`, or `pull` records an access.
- Frecency score = access count × recency multiplier.
- Recency multiplier tiers, by the age of the latest access:
  - Up to 7 days: 1.0×
  - Up to 30 days: 0.5×
  - Up to 90 days: 0.2×
  - Older: 0.05×
- A repo with no recorded access scores 0.

### Combined Ranking

Results are sorted by `match_score + frecency_score`, highest first:

- An exact match with no frecency (100 + 0 = 100) beats a substring match with moderate frecency (50 + 40 = 90).
- Within one match tier, frequently used repos come first.
- Heavy past usage can lift a substring match above an exact match in a different org.

Results are limited to 20.

## Frecency Storage

Frecency, including query frecency (see [query-frecency.md](query-frecency.md)), lives in `{cache_path}/frecency.json`, separate from the repository cache.

- Navigation commands write only this file. Each write reloads the file under a lock, records the access, and replaces the file atomically, so concurrent `work` processes never lose each other's accesses.
- Refreshing the repository cache never reads or changes it. An entry for a repo that is not in the cache stays in the file and is ignored by search; it counts again when the repo returns.
- Entries whose latest access is more than 365 days old are dropped on the next write.
- A file with an unknown version is never overwritten. Frecency is read-only for that run, and one warning says so.
- An unparsable file is renamed to `frecency.json.corrupt` with a warning, and history starts empty.

## Resolution

When a command needs a single repository (`cd`, `clone`, `edit`, and so on), it resolves the search results:

- **1 result:** used directly.
- **0 results:** error, `repository not found: <query>`.
- **1 exact match among several results:** the exact match is used, even when fuzzy matches also exist.
- **Several results, no single exact match:** ambiguous. Query frecency may settle it (see [query-frecency.md](query-frecency.md)); otherwise the behavior depends on the terminal (see [interactive-selection.md](interactive-selection.md)).

`--pick` always shows the selector when there are several results, even with an exact match or a learned query.

## List Command

`work list [query] [--filter]`

- Without a query: lists every cached repository.
- With a query: searches and shows matching repos, best match first.
- `--filter`: plain substring matching on the full slug, without fuzzy matching or frecency ranking.

## Filter Mode

With `--filter`, only substring matches are shown and they are not ranked by frecency. The results are predictable and the same for every user, which suits scripts.
