# Query-Level Frecency

## Problem

The current frecency system tracks which repositories the user accesses, but not which query led to the selection. This creates a blind spot: if the user types `work cd foo` fourteen times and picks `github.com/acme/foobar` from the selector every time, the system learns that `foobar` is popular, but it doesn't learn that `foo` means `foobar`.

The practical consequence is that short, ambiguous substrings keep triggering the interactive selector even when the user's intent is well-established. The selector is a speed bump on a well-worn path.

## Desired Outcome

When a user has repeatedly selected the same repo for a given query, the system should auto-resolve that query to the chosen repo without showing the selector. The user's established query-to-repo associations should be treated as learned preferences.

## Design

### Data Model

Add a `QueryFrecency` map to the cache alongside the existing `Frecency` map:

```json
{
  "frecency": {
    "github.com/acme/foobar": { "access_count": 14, "last_access": "..." }
  },
  "query_frecency": {
    "foo": {
      "github.com/acme/foobar": { "access_count": 14, "last_access": "..." }
    },
    "bar": {
      "github.com/acme/barlib": { "access_count": 3, "last_access": "..." }
    }
  }
}
```

The structure is `map[string]map[string]FrecencyData`, keyed first by the normalized query string (lowercased, trimmed), then by repo slug. Each entry reuses the existing `FrecencyData` struct (access count and last access time).

### Recording

Query frecency is recorded only when ambiguity is involved: either the query was auto-resolved via an existing query frecency association, or the user selected from the interactive picker. Queries that resolve unambiguously (single result or single exact match) do not record query frecency. This avoids unnecessary bookkeeping on the common hot path. If a previously unambiguous query becomes ambiguous (e.g., a new repo is added), the user builds an association in 3 selections, which is fast enough not to warrant pre-recording.

### Resolution

Query frecency participates in resolution, not search ranking. The check happens after `Resolve` returns `ErrAmbiguous` but before the interactive selector is shown:

1. `Resolve` returns `ErrAmbiguous` with multiple candidates.
2. Look up the query in `QueryFrecency`.
3. Among the ambiguous candidates, find the one with the highest query-specific frecency score (same formula: `access_count * recency_multiplier`).
4. If the top candidate's query frecency score exceeds a confidence threshold, auto-resolve to that repo.
5. Otherwise, fall through to the interactive selector as today.

This keeps query frecency out of the general search scoring path. It acts as a disambiguation override, not a ranking factor. The search results, their order, and the selector contents remain unchanged; only the "do we need to ask?" decision is affected.

### Confidence Threshold

A query-level association should only auto-resolve when the signal is strong enough that the user would be surprised if the selector appeared. The threshold has two components:

1. Minimum access count: the query must have been resolved to the top candidate at least 3 times. This prevents a single accidental selection from creating a permanent binding.
2. Dominance ratio: the top candidate's query frecency score must be at least 3x the second-highest candidate's score (if any second candidate exists in the query history). This handles the case where the user has selected different repos for the same query over time.

Both conditions must be met. If either fails, the selector is shown.

These values (3 accesses, 3x dominance) are starting points. They should be constants in the code, easy to tune if real usage suggests different thresholds.

### Query Normalization

Queries are normalized before lookup and storage: lowercased and whitespace-trimmed. No other normalization (no stemming, no slash manipulation). The query `Foo` and `foo` map to the same entry; `acme/foo` is a distinct entry from `foo`.

### Garbage Collection

During cache updates (`BuildUpdatedCache`), query frecency entries are cleaned up the same way repo frecency entries are today:

- Remove any slug from a query's map if the slug no longer exists in the repo list.
- Remove any query entry entirely if its slug map is empty after cleanup.

### Escape Hatch

If the user's preference changes (they now want `foo` to resolve to a different repo), they need a way to break the learned association. Two paths handle this naturally:

1. The selector: if the user runs the command and the system auto-resolves to the wrong repo, they can use a more specific query to reach the desired repo. That new query builds its own association. Meanwhile, not using the old query lets its recency decay.
2. Recency decay: if the user stops selecting a particular repo for a query, the recency multiplier drops over time (1.0 to 0.5 to 0.2 to 0.05), eventually falling below the confidence threshold. The selector reappears, and the user can establish a new preference.

A `--pick` flag on `cd` (and other navigation commands) could force the selector to appear regardless of query frecency, giving the user an explicit override. This flag is optional for the initial implementation but worth considering.

## Scope

### Affected Commands

All commands that call `resolveOrSelect`: `cd`, `on`, `edit`, `clone`, `browse`, `open`, `pull`.

### Not Affected

- `work list`: uses `Filter`, not `Search`/`Resolve`. No query frecency.
- Search ranking: query frecency does not alter the `match_score + frecency_score` formula. It only affects the ambiguity resolution step.

## Implementation Sketch

### Cache Changes (src/cache/)

- Add `QueryFrecency map[string]map[string]FrecencyData` field to `Cache`.
- Add `RecordQueryAccess(query, slug string)` method, parallel to `RecordAccess`.
- Add `GetQueryFrecencyScore(query, slug string) float64` method.
- Add `ResolveWithQueryFrecency(c *Cache, results []SearchResult, query string) (*Repo, error)` that wraps `Resolve` and applies the query frecency override on `ErrAmbiguous`.
- Update `BuildUpdatedCache` to preserve and garbage-collect query frecency.
- Update `Read` to initialize the map if nil (same pattern as `Frecency`).

### Command Changes (src/commands/)

- Update `resolveOrSelect` to accept the cache, call `Resolve` first, then `ResolveWithQueryFrecency` on ambiguity. Record query frecency inline when ambiguity is resolved (auto or selector).
- `recordFrecency` remains repo-level only; query frecency is handled in `resolveOrSelect`.
- Thread the cache through from each command's `RunE` to `resolveOrSelect`.

### Test Cases

- Query frecency auto-resolves after threshold is met.
- Query frecency does not auto-resolve below minimum access count.
- Query frecency does not auto-resolve when dominance ratio is insufficient.
- Dominance with runner-up: 3x ratio auto-resolves.
- No query history falls through to ambiguous.
- Single result and exact match bypass query frecency entirely.
- Query frecency recorded on ambiguous selector pick.
- Query frecency NOT recorded on clean (unambiguous) resolve.
- Garbage collection removes entries for deleted repos.
- Garbage collection removes empty query entries.
- `--pick` flag (if implemented) bypasses query frecency.
