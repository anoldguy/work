# Configuration

## Config File

- **Location:** `<platform config dir>/work/config.toml`
  - Linux: `$XDG_CONFIG_HOME/work/config.toml` (fallback `~/.config/work/config.toml`)
  - macOS: `~/.config/work/config.toml`
- **Format:** TOML
- Created automatically during first-time setup via `work setup`.

## Config Values

| Key | Description | Default |
|-----|-------------|---------|
| `work_dir` | Root directory for all cloned repos | `~/code` |
| `cache_path` | Directory where the cache file lives | `~/.local/state/work` (XDG state dir) |

### `[filters]` Section

| Key | Description |
|-----|-------------|
| `exclude_orgs` | List of org names to never sync |
| `include_orgs` | List of org names to exclusively sync (when non-empty, only these orgs are synced) |

If both are specified, `include_orgs` takes precedence over `exclude_orgs`.

### `[domains.<hostname>]` Sections

One section per GitHub domain (e.g., `github.com`, `github.enterprise.com`).

| Key | Description |
|-----|-------------|
| `is_default` | Boolean. At most one domain may be `true`. Controls path stripping behavior. |
| `favorite_orgs` | List of org names for this domain. Only these orgs are synced during cache updates unless `--all-orgs` is used. |
| `extra_orgs` | List of extra org names (e.g. orgs reachable through team or collaborator access without membership) to include in cache updates. |
| `api_url` | Optional. Base URL of the domain's REST API. Defaults to `https://api.github.com` for `github.com`, `https://api.<host>` for `*.ghe.com`, and `https://<host>/api/v3` for other GitHub Enterprise Server hosts. Must be `https` (plain `http` is accepted only for loopback hosts, which is useful for testing). Set it when the API lives behind a proxy or at a non-standard path. |

### Example

```toml
work_dir = "~/code"
cache_path = "~/.local/state/work"

[filters]
exclude_orgs = ["archived-org"]

[domains."github.com"]
is_default = true
favorite_orgs = ["my-team", "my-company"]

[domains."github.enterprise.com"]
favorite_orgs = ["corp-team"]
```

## Commands

### `work setup [domain]`

Interactive first-time setup. Prompts for work directory, tests auth for the domain, and creates the config file. Can be re-run to add additional domains.

### `work config`

Show all current configuration values in a formatted summary.

### `work config --edit`

Open the config file in the configured editor (`$WORK_EDITOR`, then `$EDITOR`, then `vi`) for direct editing. This is the escape hatch for power users who need to change values like `work_dir`, `cache_path`, or filter lists.

### `work config path`

Print the config file location on stdout, and whether it exists (`config file exists` / `config file missing`) on stderr, so `$(work config path)` yields just the path. Useful for debugging and scripting.

### `work config default [domain]`

Set which domain is the default. If no domain is specified, shows an interactive picker.

### `work favorite [domain]`

Interactive picker for selecting favorite orgs. Shows all orgs the user has access to on the specified domain and lets them toggle favorites. If no domain is specified, it uses the only configured domain, or shows an interactive domain picker when more than one is configured. With no domains configured, it fails and suggests `work setup`.

