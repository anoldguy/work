# Development Guide

## Prerequisites

- Rust (stable, edition 2024) via [rustup](https://rustup.rs/). The crate is the only implementation.

## Build

```bash
cargo build --release      # binary at target/release/work
```

## Test

```bash
cargo nextest run                  # optional; what CI runs
cargo test --doc                   # doc tests only; CI runs these separately
cargo test --test cli_about        # one integration test file
```

## Run

```bash
cargo run -- <command>     # e.g. cargo run -- list
```

## Lint and Format

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt (rewrites files)
cargo fmt --check (what CI runs)
```

Clippy runs with `-D warnings`, so any warning fails the build.


## Key Constraints

- Shell out to `git` for all git operations (no libgit2, gix, or go-git).
- Shell out to `gh` for every GitHub API call (`gh api --hostname <domain>`) and for git credentials (`gh auth git-credential`, scoped to the repo's domain). `work` never reads, stores, or forwards a token.
- stdout carries only command output and the one path line that `sh-cd`/`sh-on` print for the shell wrapper (data, never shell code). All progress, spinners, prompts, and warnings go to stderr.
- Error messages are single-line. Don't embed `\n` in error text. Put actionable hints inline with parentheses instead. Sole exception: an error that lists repos/matches (e.g. ambiguous query) prints its one-line summary followed by one `  - item` line per entry, rendered from a `Vec` the error carries (never from `\n` in the message).
- Internal program state is separate from terminal output. The terminal has ONE owner (`ui::Ui`), and it is responsible for all updates/output. From Wave 8, `clippy.toml` enforces this: printing macros, `std::io::stdout`/`stderr`, and `std::process::Command::new` are disallowed everywhere except `Ui::new`, `main`, `proc::command`, and test code.
- **MUST:** `specs/` is read-only. Do not edit any file under `specs/` without explicit user approval. If a design or implementation decision would change specified behavior, stop and ask the user first; until they approve, follow the spec as written.

## Rust Engineering Standards

Merged from the house Rust guide. These apply to all code in this crate.
When one conflicts with a Key Constraint above, the Key Constraint wins
(e.g. shelling out to `git`/`gh` is mandatory here).

### Ownership, borrowing, lifetimes (§5 of the house guide)

- Accept borrowed, general types in signatures: `&str` over `&String`,
  `&[T]` over `&Vec<T>`, `&Path`/`impl AsRef<Path>` over `&PathBuf`. Take
  ownership only when you store, send, or consume the value.
- Return owned values from constructors and transformations; return a
  reference only when it clearly borrows from `self` or an argument, and
  write the `'_` so the borrow is visible (`-> Iter<'_, T>`). The
  `mismatched_lifetime_syntaxes` lint fails CI under `-D warnings`.
- Let elision work; avoid `'static` bounds unless genuinely needed.
- **Do not `.clone()` to silence the borrow checker.** Restructure scopes,
  split borrows, `mem::take`/`replace`, or use indices first. Clones that
  are fine: `Arc::clone`/`Rc::clone` (write it explicitly), small Copy-like
  data, tests, and the per-thread `Domain`/`Org` clones the concurrent
  cache update needs to cross a `thread::scope` boundary.
- Reach for `Rc<RefCell<_>>`, `Arc<Mutex<_>>`, or `unsafe` only when shared
  ownership or interior mutability is the actual requirement, not to escape
  a design problem. `unsafe` is forbidden in this crate (see lints below).

### Idiomatic Rust (§6)

- Prefer iterator chains and expressions to index loops; gather fallible
  results with `collect::<Result<Vec<_>, _>>()`.
- **Parse, don't validate.** Domain values (`Domain`, `Org`, `RepoName`,
  `Slug`) should reject illegal states at construction so every instance is
  valid. A newtype that accepts any `String` is a stringly-typed API in
  disguise — give it a fallible constructor when the value feeds a path or
  a URL. Make illegal states unrepresentable with enums over bools/sentinels.
- Use `let ... else`, `?`, and `Option`/`Result` combinators for early
  returns; flatten nesting with let chains (1.88) and `if let` guards (1.95).
- Use explicit overflow semantics (`checked_*`/`saturating_*`/`wrapping_*`)
  where overflow matters; don't rely on plain `+`.
- `unwrap()`/`expect()`/`panic!` are for tests and violated invariants only.
  In library code return a typed error; document any `expect` with the
  invariant it relies on.
- Derive `Debug` on public types. Never let a credential reach a log or
  error chain.
- Use `impl From`/`TryFrom` for conversions, never implement `Into`
  directly. Provide `Default` where sensible; mark pure helpers `#[must_use]`
  / `const fn` where feasible.

### Lints and configuration (§4)

- Configure lints in `Cargo.toml`'s `[lints]` table, not with crate-level
  attributes in source. Target set for this crate:

  ```toml
  [lints.rust]
  unsafe_code = "forbid"
  unused_must_use = "deny"

  [lints.clippy]
  all = { level = "warn", priority = -1 }
  pedantic = { level = "warn", priority = -1 }
  module_name_repetitions = "allow"
  ```

- Fix lints; don't suppress them. If an `allow` is truly warranted, scope it
  to the single item and prefer `#[expect(..., reason = "...")]` so it warns
  once the lint stops firing. Never add `#![deny(warnings)]` to source —
  enforce `-D warnings` in CI (`make lint` already does).
- For the deliberate `as` casts in `src/ui/splash/*` (bounded color/geometry
  math), use a scoped `#[expect(clippy::cast_*)]` with a reason rather than a
  blanket allow if `pedantic` is enabled.

### Testing (§3)

- Fewer tests, each guarding behavior a user would notice. Every bug fix
  gets a regression test; new behavior gets a test at the level a user would
  notice (usually an integration test through the binary). Private helpers
  do not need their own tests. Name tests after behavior; test outputs, not
  internals or exact wording.
- Prefer `assert_matches!(value, Pattern)` (stable 1.96; `use
  std::assert_matches;` in the test module) over `assert!(matches!(...))` so
  failures print the actual value. The codebase's `panic!("expected X, got
  {other:?}")` arms can move to this.
- Use `tempfile` for filesystem tests. No sleeps, no wall-clock or network
  reliance. Integration tests use real `git` and a fake `gh` script on
  `PATH`. Use `proptest` only for parsers (`Slug`, query parsing).

### Dependencies and crate metadata (§8)

- Prefer std over crates where it suffices (`OnceLock`/`LazyLock`,
  `IsTerminal`). Minimize default features (`default-features = false`).
- Pin caret requirements with a minor version (`etcetera = "0.10"`), not a
  bare major like `"0"`/`"1"`, so a breaking 0.x bump can't slip in silently.
- Declare `rust-version` (MSRV) in `[package]` and keep edition 2024 set.
  Honor the `incompatible_msrv` clippy lint; never `allow` it.
- Run the supply-chain tools in CI: `cargo deny check` (add a `deny.toml`),
  `cargo audit`, `cargo machete`. List any skipped because a tool isn't
  installed; never report a skipped check as passed.
- Keep `main.rs` thin (it is): parse, dispatch, map errors to one line.

### Definition of done

`make check` (fmt-check, clippy `-D warnings`, test) passes, plus doc tests;
new behavior and bug fixes have tests; public items have doc comments with
`# Errors`/`# Panics` where relevant; no clone-to-appease-the-borrow-checker,
no `unsafe`, no stray `unwrap()` in library code, no new unjustified
dependency. State exactly what was run and its result.

## Release

Use cargo-dist to provide builds and push releases to github.
