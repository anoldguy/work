# Navigation Commands

## Overview

Navigation commands are the core user-facing commands. They let users quickly get to a repository — whether that means changing directories, opening an editor, or opening a browser.

## Commands

### work cd [query]

Change the shell's working directory to a repository.

- With a query: resolves the repo, clones if not present locally, changes to the repo directory.
- Without a query: changes to the work root directory.
- Records a frecency access for the resolved repo.
- Requires shell integration (the `work` shell function) because a subprocess cannot change the parent shell's directory.
- If the repo is ambiguous, shows an error with suggestions.

### work on \<query\>

Convenience command that combines `cd` and `edit`.

- Resolves the repo, clones if needed, changes to the directory, AND opens the editor.
- Records a frecency access.
- Requires shell integration.
- A query is required (unlike `cd`).

### work edit \<query\>

Open a repository in the configured editor.

- Resolves the repo, clones if needed, opens in the editor.
- Records a frecency access.
- Editor resolution: `$WORK_EDITOR` → `$EDITOR` → `vi`.
- Does NOT require shell integration (doesn't change directory).

### work clone \<query\>

Clone a repository from GitHub.

- Resolves the repo query to a specific repository.
- If already cloned (`.git` exists at the target path), does nothing.
- If not cloned, clones via git using the repo's URL from the cache.
- Authentication tokens are passed to git for private repos.
- Records a frecency access.

### work browse \<query\>

Open a repository's GitHub page in the default web browser.

- Resolves the repo, opens its URL (from the cache) in the system browser.
- Does NOT clone the repository.
- Records a frecency access.
- Uses the platform's native "open URL" mechanism (`xdg-open` on Linux, `open` on macOS).

### work open \<query\>

Open a repository's local directory in the system file manager.

- Resolves the repo, clones if needed, opens the directory in Finder/file manager.
- Records a frecency access.
- Uses the platform's native "open directory" mechanism.

## Common Behaviors

### Auto-clone

Commands that need a local copy (`cd`, `on`, `edit`, `open`) automatically clone the repository if it doesn't exist locally. Clone is idempotent — if the repo is already cloned, it's a no-op.

### Frecency tracking

All navigation commands record an access when a repo is successfully resolved. This influences future search ranking.

### Shell integration requirement

Commands that change the shell's working directory (`cd`, `on`) require shell integration. Without it, the tool prints an error directing the user to set up shell integration via `work activate`.

### Query resolution

All commands resolve queries using the search system (exact → substring → fuzzy, combined with frecency). See [search-resolution.md](search-resolution.md).
