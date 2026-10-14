# Self-Management Commands

Commands for managing the `work` CLI itself.

## Commands

### `work version`

Print the current version and exit.

- Outputs the version string (e.g., `1.0.0`)
- Version follows semantic versioning and is embedded at build/release time — no VERSION file is read at runtime

### `work about`

Display an animated about screen. See [about-easter-egg.md](about-easter-egg.md) for details.

## Self-Update

Self-update is handled by the release/distribution mechanism, not by the CLI itself. There is no `work update` self-update command. Cache updates are covered in [repository-cache.md](repository-cache.md).

## No Subcommand Behavior

Running `work` with no arguments prints help — a summary of available commands and usage. It does not silently exit.
