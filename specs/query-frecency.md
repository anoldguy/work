# Query-Level Frecency

## Purpose

Query frecency records which repo the user picks for an ambiguous query. Once a query has a clear favorite, that query resolves to it without showing the selector.

## Data Model

Query frecency lives in `frecency.json` next to repo frecency (see [search-resolution.md](search-resolution.md#frecency-storage)):

```json
{
  "version": 1,
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

`query_frecency` maps a normalized query to a map of repo slugs, each with an access count and the time of the latest access.

## Query Normalization

Queries are lowercased and trimmed of surrounding whitespace before lookup and storage. Nothing else changes: `Foo` and `foo` share an entry, and `acme/foo` is a different entry from `foo`.

## Recording

A query pick is recorded only when the query was ambiguous: either query frecency settled it, or the user picked from the selector. A query that resolves on its own (one result, or a single exact match) records no query frecency.

The pick is recorded together with the repo access, after the command succeeds.

## Resolution

Query frecency takes part in resolution, not in search ranking. It runs only when a query is ambiguous, before the selector appears:

1. Look up the query's history.
2. Score each entry with the frecency formula (access count × recency multiplier).
3. If the top entry is not among the current candidates, show the selector.
4. If the top entry passes the confidence threshold, resolve to it.
5. Otherwise, show the selector (or, without a terminal, report the ambiguity).

The search results, their order, and the selector's contents are the same with or without query frecency.

## Confidence Threshold

Both conditions must hold:

1. **Minimum count.** The top entry has at least 3 recorded picks.
2. **Dominance.** The top entry's score is at least 3× the score of the second-highest entry in the query's history, whether or not that repo is a current candidate. With no second entry, or a second entry scoring 0, this condition holds.

The values 3 and 3× are named constants.

## Changing a Preference

- `--pick` always shows the selector, and the pick is recorded, so repeated picks move the preference.
- A more specific query builds its own history.
- An entry that stops being picked decays (1.0 → 0.5 → 0.2 → 0.05) until it no longer passes the threshold, and the selector returns.

## Scope

- **Uses query frecency:** every command that resolves one repository: `cd`, `on`, `edit`, `clone`, `browse`, `open`, `pull`.
- **Does not:** `work list`, and the search ranking formula.
