# Search and Repository Resolution

## Repo Queries

Most commands accept a "repo query" — a partial or full name used to identify a repository. Queries support three levels of specificity:

- **Repo name only** (`foo`) — matches against the repository name (last segment of the slug).
- **Org/repo** (`acme/foo`) — matches against the org and repo name (last two segments).
- **Full slug** (`github.com/acme/foo`) — matches the entire slug.

Matching is case-insensitive at all levels.

## Search Algorithm

Search uses a tiered scoring system combining match quality and usage frequency.

### Match Quality Tiers

1. **Exact match (score: 100)** — The query exactly matches at the appropriate specificity level. `foo` exactly matches a repo named `foo`, but not `foobar`.
2. **Substring match (score: 50)** — The query appears as a substring of the repo name or full slug.
3. **Fuzzy match (score: normalized)** — The query matches via fuzzy/subsequence matching. Scores are normalized from the fuzzy matcher's raw scores.
4. **No match** — Repos with no match at any tier are excluded from results.

### Frecency Scoring

Each repository has a "frecency" score based on usage patterns:

- Every time the user navigates to, edits, or opens a repo, an access is recorded.
- Frecency score = access count x recency multiplier.
- Recency multiplier tiers:
  - Last 7 days: 1.0x
  - Last 30 days: 0.5x
  - Last 90 days: 0.2x
  - Older: 0.05x
- Repos never accessed have a frecency score of 0.

### Combined Ranking

Results are sorted by `match_score + frecency_score` (descending). This means:

- An exact match with no frecency (100 + 0 = 100) beats a substring match with moderate frecency (50 + 40 = 90).
- Among repos with the same match tier, frequently-used repos appear first.
- Heavy past usage can promote a substring match above an exact match in a different org.

Results are limited to 20.

## Resolution

When a command needs a single repository (cd, clone, edit, etc.), the search results are resolved:

- **1 result**: Used directly.
- **0 results**: Error — "repository not found: \<query\>"
- **1 exact match among multiple results**: The exact match is used, even if fuzzy matches also exist.
- **Multiple results, no single exact match**: Ambiguous — behavior depends on context (see [interactive-selection.md](interactive-selection.md)).

## List Command

`work list [query] [--filter]`

- Without a query: lists all cached repositories (useful for shell completion).
- With a query: searches and displays matching repos, best matches first.
- `--filter` flag: uses simple substring matching instead of fuzzy search, disables frecency ranking.

## Filter Mode

When `--filter` is used, only substring matches are shown (no fuzzy matching), and results are not ranked by frecency. This is useful for scripting or when the user wants predictable, non-personalized results.
