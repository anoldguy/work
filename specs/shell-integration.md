# Shell Integration

## Why Shell Integration Exists

A subprocess cannot change its parent shell's working directory. `work cd` and `work on` change the user's shell directory, so they run through a shell wrapper function.

## Activation

`work activate [shell]`

- Prints the wrapper script to stdout, for the user's shell config to load.
- Detects the shell from `$SHELL` when none is given.
- Supported shells: bash, zsh, fish.
- The script defines a `work` function:
  - `work cd …` runs `work sh-cd …`, and on success changes to the directory it printed.
  - `work on …` runs `work sh-on …`, and on success changes to the directory it printed, then runs `work sh-editor`.
  - Every other command runs the binary directly.
- The wrapper captures the output first and acts on it second. If the hidden command fails, the wrapper returns its exit status and the directory does not change.
- The wrapper uses the shell's ordinary `cd`, so the user's own `cd` functions and hooks still run.

### Installation

Users add one of these to their shell config:

- Bash (`~/.bashrc`): `eval "$(work activate bash)"`
- Zsh (`~/.zshrc`): `eval "$(work activate zsh)"`
- Fish (`~/.config/fish/config.fish`): `work activate fish | source`

A shell that is already running keeps the wrapper it loaded. After upgrading `work`, restart the shell.

## Wrapper Commands

These hidden commands exist only for the wrapper:

- **`work sh-cd [query] [--pick]`** does everything `cd` does except change directory: it resolves, clones, and records the access. It prints exactly one line to stdout, the absolute path of the target directory, and nothing else.
- **`work sh-on <query> [--pick]`** does the same for `on`.
- **`work sh-editor`** opens the editor (`$WORK_EDITOR` → `$EDITOR` → `vi`) on the current directory, with the terminal attached. It resolves nothing and records nothing.

The output of `sh-cd` and `sh-on` is data, never shell code. Progress, prompts, the selector, and warnings go to stderr. A target path containing a newline, or one that is not valid UTF-8, is an error.

Running `work cd` or `work on` without the wrapper fails with a message pointing to `work activate`.

## Completions

`work completions [shell] [--install]`

- `work completions [shell]` prints the completion script to stdout. The shell is detected when none is given.
- `work completions [shell] --install` writes it to the shell's completion directory.

### Completion Install Locations

- Fish: `~/.config/fish/completions/work.fish`
- Bash: `~/.local/share/bash-completion/completions/work` (or Homebrew's completion dir on macOS)
- Zsh: `~/.local/share/zsh/site-functions/_work` (or Homebrew's completion dir on macOS)

Repository names are completed from the cache, without network access.

## Shell Detection

The shell comes from the `$SHELL` environment variable. An unrecognized shell falls back to bash. Recognized shells: bash, zsh, fish.

## No-Subcommand Help

Running `work` with no arguments prints a help summary. Help output and shell completions are how users discover commands; the hidden wrapper commands appear in neither.
