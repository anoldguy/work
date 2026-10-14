# Shell Integration

## Why Shell Integration Exists

A subprocess cannot change its parent shell's working directory. Commands like `work cd` and `work on` need to change the user's shell directory, so they require a shell wrapper function that intercepts these commands and evaluates their output in the current shell context.

## Activation

`work activate [shell]`

- Outputs a shell script to stdout, intended to be eval'd in the user's shell config.
- Auto-detects the shell from `$SHELL` if not specified.
- Supported shells: bash, zsh, fish.
- The output defines a `work()` shell function that:
  - For shell-integrated commands (`cd`, `on`): runs the binary with a `sh-` prefix and eval's the output.
  - For all other commands: passes through to the binary directly.

### Installation

Users add one of these to their shell config:

- Bash (`~/.bashrc`): `eval "$(work activate bash)"`
- Zsh (`~/.zshrc`): `eval "$(work activate zsh)"`
- Fish (`~/.config/fish/config.fish`): `work activate fish | source`

## Shell-Integrated Commands

Commands that need to modify parent shell state output shell code to stdout (instead of executing directly). The shell wrapper function eval's this output.

Currently shell-integrated commands: `cd`, `on`.

The shell scripts query the binary for the list of integrated commands at load time (e.g., `work commands --sh`). This is an internal mechanism — not a user-facing command for discoverability. The list drives the wrapper function's intercept logic.

If a user tries to run a shell-integrated command without the wrapper function installed, the tool outputs an error directing them to `work activate`.

## Completions

`work completions [shell] [--install]`

- `work completions [shell]` — Output completion script to stdout. Shell is auto-detected if not specified.
- `work completions [shell] --install` — Install completions to the appropriate system directory.

### Completion Install Locations

- Fish: `~/.config/fish/completions/work.fish`
- Bash: `~/.local/share/bash-completion/completions/work` (or Homebrew's completion dir on macOS)
- Zsh: `~/.local/share/zsh/site-functions/_work` (or Homebrew's completion dir on macOS)

Shell is auto-detected if not specified.

## Shell Detection

The tool detects the current shell from the `$SHELL` environment variable. Falls back to bash if unrecognized. Recognized shells: bash, zsh, fish.

## No-Subcommand Help

Running `work` with no arguments prints a help summary of available commands. There is no user-facing `work commands` command for discoverability — help output and shell completions cover that. The internal `work commands --sh` mechanism exists solely for shell integration scripts (see above).
