# Project Goal

A CLI tool that organizes and navigates GitHub repositories by mirroring the `domain/org/repo` hierarchy locally. Jump to code faster; spend less time wondering where you cloned that thing three months ago.

## What It Does

- Clones repositories into a structured directory tree: `~/code/[domain/]org/repo`
- Navigates to repositories by fuzzy name, ranked by usage patterns (frecency)
- Caches repository metadata locally for fast search and tab completion
- Supports multiple GitHub domains (github.com + GitHub Enterprise)
- Integrates with the user's shell to enable directory-changing commands

## What It Doesn't Do

- No daemon or background process
- No cloud sync
- No database — just a flat cache file and config
- No custom auth — delegates to GitHub CLI (`gh`)
- No git library — shells out to `git` for all operations
