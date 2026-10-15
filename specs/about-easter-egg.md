# About Easter Egg

## Command

`work about`

## Logo and Text

```
 ██╗    ██╗ ██████╗ ██████╗ ██╗  ██╗
 ██║    ██║██╔═══██╗██╔══██╗██║ ██╔╝
 ██║ █╗ ██║██║   ██║██████╔╝█████╔╝
 ██║███╗██║██║   ██║██╔══██╗██╔═██╗
 ╚███╔███╔╝╚██████╔╝██║  ██║██║  ██╗
  ╚══╝╚══╝  ╚═════╝ ╚═╝  ╚═╝╚═╝  ╚═╝

               v0.1.0
     Made with love by anoldguy
      github.com/anoldguy/work
```

The version string should match the current running version.

## Behavior

Displays a full-screen animated about screen with project branding, version, and attribution, rendered at 60fps.

- The animation style is chosen randomly by default. A hidden `--style` flag
  selects a specific style by name instead (mainly useful for development and
  screenshots); an unrecognized name fails with an error listing the valid
  names.
- Available styles, in registry order:
  - `classic`: the logo with a rainbow gradient that scrolls one row per 150ms.
  - `decode`: the logo starts as scrambled glyphs and locks into place column by
    column over 1.6s, then a bright diagonal shimmer sweeps across it every 3s.
    The footer types itself in after the lock completes.
  - `dropin`: each logo column falls in from above on a spring, staggered left
    to right, overshoots once and settles; the footer then springs up from below.
- Every style is time-based rather than frame-counted, so a dropped frame skips
  ahead instead of slowing the animation. Styles that use randomness derive it
  from a seed so tests can render deterministic frames.
- Renders in the alternate screen buffer (does not pollute scrollback)
- Includes a blinking "press any key to exit" prompt
- Centers content in the terminal
- Output goes to stderr
- Without a terminal on stderr (or with `TERM=dumb`), prints the static logo and footer shown above to stderr and exits instead of animating.

## Purpose

A fun, non-critical easter egg. Not part of any workflow.
