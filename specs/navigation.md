# Navigation Commands

## Overview

Navigation commands get the user to a repository: change into its directory, open it in an editor, a browser, or a file manager, or clone it.

## Commands

### work cd [query]

Change the shell's working directory to a repository.

- With a query: resolves the repo, clones it if it is not present locally, and changes to its directory.
- Without a query: changes to the work directory, creating it if it is missing.
- Requires shell integration (see [shell-integration.md](shell-integration.md)).

### work on \<query\>

`cd` followed by the editor.

- Resolves the repo, clones it if needed, changes to its directory, then opens the editor there.
- Requires shell integration.
- A query is required.

### work edit \<query\>

Open a repository in the editor.

- Resolves the repo, clones it if needed, and opens it in the editor.
- Editor resolution: `$WORK_EDITOR` → `$EDITOR` → `vi`.
- Does not need shell integration.

### work clone \<query\>

Clone a repository and print its path.

- Resolves the query to one repository.
- If it is already cloned (`.git` exists at the target path), does nothing.
- Otherwise, clones it with git from the clone URL in the cache or the adopted repositories. git authenticates through `gh` (see [authentication.md](authentication.md)).

### work browse \<query\>

Open a repository's GitHub page in the default web browser.

- Resolves the repo and opens its web URL from the cache or the adopted repositories.
- Never clones.
- Uses the platform's "open URL" mechanism (`xdg-open` on Linux, `open` on macOS).

### work open \<query\>

Open a repository's local directory in the file manager.

- Resolves the repo, clones it if needed, and opens the directory.
- Uses the platform's "open directory" mechanism.

## Common Behaviors

### Auto-clone

`cd`, `on`, `edit`, `clone` and `open` clone the repository when it is not present locally. Missing parent directories are created; if the clone fails, the directories it created are removed again.

### Frecency tracking

Every navigation command records an access once its action succeeds. A failed clone, editor, or opener records nothing.

### Shell integration requirement

`cd` and `on` change the shell's working directory, so they need the shell wrapper. Without it, the command fails with a message pointing to `work activate`.

### Query resolution

Every command resolves its query with the search system (exact → substring → fuzzy, combined with frecency). See [search-resolution.md](search-resolution.md).

An `org/repo` or `domain/org/repo` query with no exact match is looked up on GitHub, so every command here reaches any repository the user can read, cached or not. See [search-resolution.md](search-resolution.md#lookup-on-github).
