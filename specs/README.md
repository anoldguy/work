# Work CLI Specifications

Outcome-based specifications for the `work` CLI tool — a command-line tool for
organizing and navigating GitHub repositories.

These specs describe WHAT the tool does, not HOW it's built. They are
language-agnostic and implementation-agnostic.

## Specification Files

| Spec | Covers |
|------|--------|
| [GOAL.md](GOAL.md) | Project purpose and scope |
| [directory-structure.md](directory-structure.md) | Where repos live on disk, multi-domain paths, default domain |
| [configuration.md](configuration.md) | Config file, domains, filters, favorites, setup, config commands |
| [authentication.md](authentication.md) | Token resolution, gh CLI integration, auth commands |
| [repository-cache.md](repository-cache.md) | Cache contents, staleness, update from GitHub API |
| [search-resolution.md](search-resolution.md) | Repo queries, fuzzy search, frecency, disambiguation |
| [query-frecency.md](query-frecency.md) | Query-level frecency: learned query-to-repo associations for ambiguous queries |
| [navigation.md](navigation.md) | cd, on, edit, clone, browse, open |
| [sync.md](sync.md) | pull, sync (fetch + safe fast-forward) |
| [shell-integration.md](shell-integration.md) | Activation scripts, completions, shell detection |
| [self-management.md](self-management.md) | Version, about |

## Key Concepts

- **Repo query**: A partial or full name used to identify a repository (e.g., `foo`, `acme/foo`, `github.com/acme/foo`)
- **Work directory**: Root directory where all repos are cloned (default: `~/code`)
- **Default domain**: The GitHub domain whose repos get shorter paths (`org/repo` instead of `domain/org/repo`)
- **Frecency**: A ranking signal combining frequency and recency of access
- **Shell integration**: A shell wrapper function that enables commands like `cd` to change the parent shell's directory
