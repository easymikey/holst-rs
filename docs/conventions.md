# Conventions

Code rules. The board protocol is in [board-protocol.md](board-protocol.md), what the tools do is in the [readme](../readme.md). A rule a tool can check is checked by the tool; the rest is checked in review.

## 1. Names

- A type is a domain term (`Board`), the term with a suffix below, or the term narrowed by a plain word (`SessionError`). A kind of an existing term is `<Qualifier><Term>`, never a new word.
- Commands, flags, MCP tools, arguments and JSON keys use the same terms. A rename changes them with the code, with no alias.
- Full words, no abbreviations but std's; no parameter named `data`, `info`, `ctx`, `cfg`, `opts`, `tmp` or `val`.
- A field does not repeat its owner's name (`Board.owner`). A time field is `<event>_at`; milliseconds are `millis`.
- Getters are nouns (`title()`), predicates start with `is_`, `has_` or `can_`. Conversions follow `as_`, `to_`, `into_`, `from_`.
- Server words (paths, JSON keys, socket messages) live only in the wire modules `protocol` and `remote`.
- Suffixes: `Error` an error enum, `Plan` what to do before any effect, `Request` and `Response` a call and its answer, `Id` an identifier from the server, `Args` the arguments of one command, `Settings` flags and environment, `Patch` a partial change.
- Banned: the suffixes `Manager`, `Handler`, `Helper`, `Util`, `Wrapper`, `Info`, `Data`, `Params`, `Options`, `Outcome`; `fn get_*`; own `macro_rules!`; `#[allow]`; glob imports; `super::` paths; `let _ =`; `let ... else`; bare `as`.

## 2. Variants

A variant's data is nothing (unit), one value (`V(T)`), or named fields. Two or more positional values never. A tuple is not a field, a payload or a public return; a parser of bytes returns `Parsed { value, rest }`.

## 3. Code style

- Pure core, effects at the edge: a pure module returns a value or a plan, an interpreter carries it out. Only the output module prints.
- No `else if` chains, `return`, `loop`, `while` or `for`; use iterators, combinators and `match`. A construct that needs one carries a `// imperative: <reason>` line. A `let mut` needs the same line.
- `match` is exhaustive, with no `_ =>` arm. `if let` only where the `match` would have one empty arm.
- No `unwrap`, `expect`, `panic!`, indexing or string slicing. Errors travel as values.
- Make illegal states unrepresentable: no `bool` beside an `Option`, no `Option<Option<_>>`.
- Limits: a function within 60 lines, at most 4 parameters, nesting under 4, a file within 800 lines.
- No magic values; no macros except the two kept in `holst-macros`; `pub(crate)` unless another crate needs it; no shims, delete rather than deprecate.
- Files are written through `holst_core::fs`: a sibling partial file, then `rename`; a secret is created with mode 0600.
- Comments are few. `///` goes on public items whose signature does not say it all, one line, a third-person verb. `//` says why, never what. Fixed forms: `SAFETY:`, `PROTOCOL:` (names the section of `docs/board-protocol.md`), `GUARD:`, `// imperative:`.

## 4. Tests

- One behaviour, one test, named after the behaviour. A defect gets a failing test first.
- The pure core is tested by value in and value out (`rstest`, `proptest`, `insta`). Shells are tested through the binary against the fake Holst, once per surface.
- Integration tests live in `crates/*/tests/main.rs`, one binary per crate, and see only the public API. Unit tests sit at the bottom of the file they test.
- A test has no branch and no `unwrap`; it returns `Result`. Tests touch no real home and no real network. Live writes happen by hand on boards named `zz-test-*`.

## 5. Errors and output

- Each crate has one boundary `Error`, `thiserror` enums with `#[error]` text on every variant. No `anyhow`, no `Box<dyn Error>`.
- A user-facing error is a plain sentence naming the cause and, where the user can act, what to try. No OS codes or third-party text; an HTTP status shows only under `--verbose`.
- Results go to stdout, errors, hints and logs to stderr. `holst-mcp` writes only protocol messages to stdout.
- Exit codes follow BSD `sysexits` and are chosen in `exit.rs` (64 usage, 65 data, 66 not found, 69 unavailable, 73 file exists, 74 IO, 75 retry, 77 permission or `--yes` needed, 78 config). `holst help exit-codes` lists them.

## 6. Newtypes and units

A newtype exists for a rule, a unit or a server identity. Parse once at the edge. Every `as` cast sits in a named conversion. A time is a `Duration` or a `Timestamp`. A closed set is an enum.

## 7. Checks

Lints live only in `Cargo.toml` `[workspace.lints]`; no `clippy.toml`. Format and lint run on `nightly-2026-05-22`; build and tests on the toolchain in `rust-toolchain.toml`. Spelling: `cargo spellcheck` with `.config/spellcheck.toml` and `spellcheck.dic`.
