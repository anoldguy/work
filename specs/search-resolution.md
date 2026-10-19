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
- Refreshing the repository cache never reads or changes it. An entry for a repo that is neither cached nor adopted stays in the file and is ignored by search; it counts again when the repo returns.
- Entries whose latest access is more than 365 days old are dropped on the next write.
- A file with an unknown version is never overwritten. Frecency is read-only for that run, and one warning says so.
- An unparsable file is renamed to `frecency.json.corrupt` with a warning, and history starts empty.

## Resolution

When a command needs a single repository (`cd`, `clone`, `edit`, and so on), it resolves the search results over the cached and adopted repositories (see [repository-cache.md](repository-cache.md#adopted-repositories)):

- **1 result:** used directly.
- **0 results:** error, `repository not found: <query>`. After a lookup on GitHub (see [Lookup on GitHub](#lookup-on-github)), the error adds a parenthesized hint to check access with `gh auth status`.
- **1 exact match among several results:** the exact match is used, even when fuzzy matches also exist.
- **Several results, no single exact match:** ambiguous. Query frecency may settle it (see [query-frecency.md](query-frecency.md)); otherwise the behavior depends on the terminal (see [interactive-selection.md](interactive-selection.md)).

`--pick` always shows the selector when there are several results, even with an exact match or a learned query.

### Lookup on GitHub

`cd`, `on`, `edit`, `clone`, `browse` and `open` reach any repository the user can read, not only cached ones. An `org/repo` or `domain/org/repo` query with no exact match among the cached and adopted repositories is looked up on GitHub before it is resolved. Substring and fuzzy matches do not prevent the lookup: `acme/foo` is looked up even when `acme/foobar` is cached.

- The lookup is `gh api --hostname <domain> repos/<org>/<repo>`. gh follows redirects, so a renamed or transferred repository is found under its current name.
- An `org/repo` query is looked up on every configured domain at once. A `domain/org/repo` query is looked up only on that domain, and only when it is configured.
- A `repo` query is never looked up.
- A query whose segments are not a valid domain, org, and repository name is never looked up. It resolves against the cached and adopted repositories without any `gh` call.
- `pull` and `sync` never look anything up. They resolve against the cached and adopted repositories only.

The lookup decides resolution as follows:

1. **Found on at least one domain.** The repositories found are the only candidates; known non-exact matches are set aside. One candidate is used directly. Several (one per domain) are ambiguous and go to the selector or the ambiguity error (see [interactive-selection.md](interactive-selection.md)). Each candidate's owner, name, clone URL and web URL come from GitHub's answer and are validated like listing entries (see [repository-cache.md](repository-cache.md#error-behavior)); an answer that fails validation counts as a failed lookup. The selected repository, unless it is cached, is adopted (see [repository-cache.md](repository-cache.md#adopted-repositories)); the first time, `adopted <slug>` goes to stderr.
2. **Found nowhere, no domain refused access, and the query has known matches.** Those matches are resolved as usual, as if no lookup had happened. Each lookup that failed for a reason other than "not found" adds one warning.
3. **Found nowhere, and a domain refused access or nothing known matches.** The command fails with one line:
   - Access refused (HTTP 401, or HTTP 403 other than a rate limit, including an organization's SAML SSO enforcement) on any domain: `cannot read <slug>: <gh's message>`, with a hint to run `gh auth status --hostname <domain>` and, when gh's message names SAML, to authorize the token for the organization. The first such domain in domain-name order is reported.
   - Otherwise, a lookup that failed for another reason (network, server error, rate limit, not logged in, an invalid answer): `cannot look up <slug>: <gh's message>`.
   - Otherwise (every domain answered "not found"): `repository not found: <query>`, with a hint naming `gh auth status --hostname <domain>`, or `gh auth status` when several domains were looked up.

Once adopted, the repository is an exact match for its slug, so resolving it again makes no lookup.

## List Command

`work list [query] [--filter]`

- Without a query: lists every cached and adopted repository.
- With a query: searches and shows matching repos, best match first.
- `--filter`: plain substring matching on the full slug, without fuzzy matching or frecency ranking.

## Filter Mode

With `--filter`, only substring matches are shown and they are not ranked by frecency. The results are predictable and the same for every user, which suits scripts.
