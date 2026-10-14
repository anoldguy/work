# Interactive Selection

## Purpose

When a repo query is ambiguous (multiple matches, no single exact match), the tool's behavior depends on whether it is running in an interactive terminal or in a scripted/piped context.

## Interactive Mode (TTY)

When stdin and stderr are both connected to a terminal, ambiguous queries present a TUI selector instead of an error. The selector shows all matching repositories ranked by score, and the user picks one.

- Typing filters the list: printable characters narrow it to entries containing the typed text (case-insensitive), keeping score order. Backspace edits the filter.
- Up/Down arrows (or Ctrl-p/Ctrl-n) move the selection. Letters such as `j`/`k` are not navigation keys, because they're part of the filter.
- Enter selects the highlighted entry. Escape cancels. Ctrl-C aborts the command.
- In multi-select pickers (for example favorite orgs), Tab toggles the highlighted entry, and selections survive filter changes.
- If the user cancels, the command falls back to the same error as non-interactive mode.
- The selector renders to stderr so it does not interfere with stdout-based output (e.g., shell eval commands).

## Non-Interactive Mode (piped/scripted)

When stdin or stderr is not a terminal (piped input, redirected stderr, cron, scripts), ambiguous queries produce an error listing the matching repositories: a one-line summary followed by one `  - <slug>` line per match. The user must re-run with a more specific query.

## Applies To

All commands that resolve a single repository from a query: `cd`, `on`, `edit`, `clone`, `browse`, `open`, `pull`.

## Rationale

Interactive selection eliminates the frustrating "ambiguous query" → retype cycle for terminal users, while preserving predictable, scriptable error behavior for automation.
