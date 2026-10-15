# Interactive Selection

## Purpose

When a repo query is ambiguous (several matches, no single exact match, and no learned query preference; see [query-frecency.md](query-frecency.md)), the behavior depends on whether `work` runs in a terminal or in a script.

## Interactive Mode (TTY)

When stdin and stderr are both connected to a terminal, ambiguous queries present a TUI selector instead of an error. The selector shows all matching repositories ranked by score, and the user picks one.

- Typing filters the list: printable characters narrow it to entries containing the typed text (case-insensitive), keeping score order. Backspace edits the filter.
- Up/Down arrows (or Ctrl-p/Ctrl-n) move the selection. Letters such as `j`/`k` are not navigation keys, because they're part of the filter.
- Enter selects the highlighted entry. Escape cancels. Ctrl-C aborts the command.
- In multi-select pickers (for example favorite orgs), Tab toggles the highlighted entry, and selections survive filter changes.
- If the user cancels, the command falls back to the same error as non-interactive mode.
- The selector renders to stderr so stdout carries only command output (such as the path `sh-cd` prints for the shell wrapper).

## Non-Interactive Mode (piped/scripted)

When stdin or stderr is not a terminal (piped input, redirected stderr, cron, scripts), ambiguous queries produce an error listing the matching repositories: a one-line summary followed by one `  - <slug>` line per match. The user must re-run with a more specific query.

## Applies To

All commands that resolve a single repository from a query: `cd`, `on`, `edit`, `clone`, `browse`, `open`, `pull`.

## Rationale

In a terminal, the user picks instead of retyping. In scripts, the outcome is a predictable error.
