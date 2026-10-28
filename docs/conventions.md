# Conventions

How code is written. Code rules only: what the words mean is in `GLOSSARY.md`, how the Holst board protocol works is in [board-protocol.md](board-protocol.md), what the tools do is in the [readme](../readme.md). A rule that a tool can check is checked by the tool (§ 7); the rest is checked in review.

The layers the rules refer to:

- **Pure core**: the modules of `holst-core` that do no I/O (ids, urls, trees, changes, board text, geometry, layout, caches' plans, error values). They take values and return values. The list is `PURE` in the style scan (an internal check, not shipped).
- **Effects as data**: a pure module returns a description of what to do (an `ApiRequest`, a cache `Plan`, a socket plan); an interpreter at the edge carries it out (`RequestInterpreter` with `RequestSender` or `RequestReporter`).
- **Interpreters and transport**: `Transport`, `Clock`, `IdSource`, the session store, `fs`, the board socket. The only code in `holst-core` that touches the network, the disk, the clock or randomness; each sits behind a trait or a function a test replaces.
- **Shells**: `holst-cli` (the `holst` binary) and `holst-mcp` (the MCP server on stdio). They parse input, call the core, render the result for their surface, and choose the exit.
- **Macros**: `holst-macros`, procedural macros only.

## 1. Names

- **Types are glossary terms**: the term itself (`Board`), the term with a suffix below, its plural for a collection, or the term narrowed by a plain word that says which case it is (`SessionError`). A type no term covers is listed under the glossary's Excepted words. A word under a term's _Avoid_ is never a type, field, variant or config key. A concept with no term is not named in code: it is worked out and added to `GLOSSARY.md` first.
- **Command names, flags, MCP tools and arguments, JSON keys and values take the term too**; a rename changes them with the code, with no alias that keeps an old name alive. Shortcuts for typing speed are welcome and are not shims: short flags (`-h`, `-o`, `-y`) and short command aliases (`ws`, `b`, `ls`), one consistent set across commands. Arguments are named after their command or tool (`snapshot: SnapArgs`, `edit: &EditArgs`), never `request`, `command`, `flags` or `arguments`.
- **Missing and unparsable values**: an error variant for a missing argument is `No<X>`; for a value that does not parse as a term, `Not<Term>`.
- **Times**: a field or parameter holding a time is `<event>_at`; milliseconds are `millis`, never `ms`.
- **Server words** (the glossary's "Server words") appear outside the wire modules only in a `serde` attribute or a wire body.
- **The prefix rule: a kind of an existing term is `<Qualifier><Term>`, never a new word**: a kind, part or side of `Board` is a `…Board`, not a synonym (`Canvas`, `Sheet`); a kind of `Error` is a `…Error`. Check the glossary before a name is proposed; a new word is for a new concept only, and a new concept needs a glossary term.
- **Name by content**: a name says what the value is or holds, not where it is used or how it got there. A type that exists only to carry a result has the name of that result.
- **Std names and C-CONV**: a function that does what a std one does has the std name (`len`, `is_empty`, `next`, `from_fn`). Conversions follow the Rust API guidelines: `as_` is a free borrow or view (`as_str`), `to_` an expensive copy, `into_` consumes (`into_parts`), `from_` builds from one value (`from_raw`). A getter is the noun (`title()`), never `get_title()`; a predicate starts with `is_`, `has_` or `can_`.
- **Full words**: no abbreviations but the ones std uses; no parameter named `data`, `info`, `ctx`, `cfg`, `opts`, `tmp`, `val` or `thing` (the full list is under Banned).
- **Fields, values and functions** use the term in snake_case (`board_id`). Inside its owner a field takes the short word, since the owner qualifies it (`Board.title`). Functions are verbs, types are nouns, every name is whole words.
- **The field rule**: a field does not repeat its type's name or a word its owner already says (`Board.owner`, not `board_owner`; `Session.expires_at`, not `session_expires_at`). A field holding its owner's subject or named by one whole glossary term is fine. A parameter for a time is named by what it is (`sent_at`, `expires_at`) or is a `clock`, never `now`.
- **Crates**: packages `holst-<name>`, the library `holst_core` in code. A type is named for what it is, not for where it lives: `holst_core::board::Board`, not `board::BoardType`. The error enums of the core are named `<Domain>Error` (`HttpError`), never a bare noun under `error::`; the crate's boundary type is `Error`.
- **The wire seam**: the words of the Holst server (paths, JSON keys, message kinds of the board socket, the shapes of its errors) live only in the core's wire modules (`protocol` and `remote`). The shells and the pure core name no wire word; they name the glossary term.

Suffixes that mean one thing everywhere:

| suffix | means | example |
|---|---|---|
| `Error` | an error enum | `holst_core::Error`, `holst_cli::Error` |
| `Report` | what is told about something that happened | `RetryReport` |
| `Policy` | limits and retries | `RetryPolicy` |
| `Kind` | a small enum that classifies another type, as `io::ErrorKind` does | `ErrorKind` |
| `Plan` | what to do, computed before any effect (effects as data) | `CachePlan` |
| `Request` | a described call, not yet sent | `ApiRequest`, `HttpRequest` |
| `Response` | what came back from the wire | `HttpResponse` |
| `Interpreter` | a trait whose implementations carry a description out | `RequestInterpreter` |
| `State` | the state of a machine (a socket, a form) | `BoardFileState` |
| `Key` | what names a stored or cached item | `CacheKey` |
| `Index` | a position in one index space | `ObjectIndex` |
| `Id` | an identifier the server gave | `BoardId` |
| `Name` | a name that picks one of a set | `SurfaceName` |
| `Args` | the arguments of one command, a struct with `CommandArgs` that the command line and the MCP tool both fill | `LoginArgs` |
| `Settings` | what flags and environment configure for a run | `Settings` |
| `Patch` | a partial change: every field an `Option`, built by struct update | `SettingsPatch` |

**Banned**:
- the suffixes `Refused`, `Failure`, `Outcome`, `Fault`, `Problem`, `Rejected` and a per-operation `*Failed`: an answer that failed is the past tense with an `Err`, or `Not<verb>ed` (`NotSent`, `NotSaved`);
- the type suffixes `Props`, `Tuning`, `Sync`, `Scratch`, `Info`, `Inputs`, `Values`, `Flags`, `Params`, `*Options`, `Data`, `Manager`, `Handler`, `Helper`, `Util`, `Utils`, `Wrapper`, `Holder`, `Draw`, `Slot`; `Type` only where std or the domain spells one word with it; `Kind` only on a small enum that classifies another type, as `io::ErrorKind`;
- the word `row` in any name; the prefix `Ui`; `Cfg` and `Ctx` in a name; the word `apply` as a verb in a function or module name (say what the change does);
- `fn get_*` (a getter is the noun: `title()`), predicates `should_`, `wants_`, `needs_` (use `is_`, `has_`, `can_`), `maybe`, `x_to_y` functions, `fn sync_*`;
- abbreviations (`msg`, `err`, `pos`, `dur`, `proto`, `req`, `resp`, `ws`, `tmp`; allowed: `buf`, `px`, and the established `Id`, `Url`, `Json`, `Svg`, `Png`, `Http`), and the parameter names `data`, `info`, `ctx`, `cfg`, `opts`, `options`, `idx`, `tmp`, `res`, `val`, `thing`, `stuff`, `params`, `args`, `props`, `w`, `h`, `n`, `i`, `e`, `s`, `p`;
- own `macro_rules!`; `#[allow]`; glob imports (`use x::*`, including `use Enum::*` and in tests); `super::` paths (use `crate::`, tests included); `let _ =`; `let … else`; `as` outside a named conversion (§ 6).

The code breaks this list in places the rename step fixes (types ending `Options` and `Outcome`, `apply` in `change::apply`, `reason: String` payloads); they are not exceptions.

## 2. Variant shapes

A variant's data takes the shape that names it best:

| the variant's data | shape | std precedent | holst examples |
|---|---|---|---|
| nothing | unit `V` | `Ordering::Less`, `Option::None` | `Error::StdoutClosed`, `Error::Cancelled`, `Error::NoPng` |
| one value whose type says what it is | one-field tuple `V(T)` | `IpAddr::V4(Ipv4Addr)`, `Bound::Included(T)` | `Carried::Reported(ApiRequest)`, `Error::Holst(holst_core::Error)` |
| one plain value whose role only a name gives | named field `V { name: T }` | none | `Error::NoInput { what }` |
| two or more values that meet only in this variant | named fields `V { a: A, b: B }` | `Prefix::UNC(&OsStr, &OsStr)` needs docs | `HttpError::Status { status, .. }` |
| a group that lives beyond the match | a struct of its own, then `V(T)` | `SocketAddrV4` | `Error::Unconfirmed(Unconfirmed)` |
| two or more positional values | never | | |

Never a tuple `(A, B)` as a field, a payload or a return of a public function. A tuple struct holds one value (a newtype); a `PhantomData` marker beside it does not count. This applies to every enum in the workspace, the shells' own included.

Two cases. **The one exception**: `CheckAll` (independent checks that report every failure at once) returns a tuple, which the caller destructures at once; the tuple is never stored or passed on. **A parser of bytes** does not return `(value, rest)`: it returns `Parsed { value, rest }`.

## 3. FP-like code

- **FP-like, stricter than the clippy default.** No `else if` chains (use `match`) and no `if` used for an effect with an `else` (but the one-arm forms below); a two-arm `if cond { a } else { b }` expression whose arms are values is allowed and preferred over `bool` riddles (`cond.then_some(()).map_or(..)`, `Some(x).filter(..).map_or_else(..)`, `match cond.then_some(())`). No `return`, `loop`, `while`, `for` and no empty match arms; data transforms go through iterators, combinators and `match`. A function a caller iterates returns `impl Iterator`. A construct the language cannot express without one of these (an async retry loop, a read loop on a socket) carries a `// imperative: <reason>` line on it or directly above; the style scan accepts nothing else.
- **`let mut` is the same exception.** A `let mut` stays only where a loop-free form is impossible, or is measurably slower: the `// imperative:` line says which, and for the second case names the benchmark and the measured ratio (`// imperative: benches/pure.rs ids, 1.8x slower as a fold`). A `let mut` with no such line is a defect.
- Compute first, then do the effect in one line at the edge: a pure module returns a plan or a value; only an interpreter, a shell's `main` path and the output module touch IO.
- `match` by default (clippy's `option_if_let_else`, `single_match_else`). `if let` is allowed exactly when the `match` would have one arm that does nothing (`match x { Some(v) => f(v), None => {} }` becomes `if let Some(v) = x { f(v) }`), with no `else`; `let … else` is banned. **The one-arm form of an effect** is `if let` or `if cond { effect }` with no `else`, for effects only (a value is never computed by an `if`); never `Some(x).into_iter().for_each(..)`, `.filter(..).into_iter().for_each(..)` or `cond.then(|| effect)` for an effect. A `match` over any enum, foreign ones included, is exhaustive, with no `_ =>` arm (`wildcard_enum_match_arm` is on everywhere; the variants of a foreign enum are listed too), so a new variant breaks the build where it matters.
- **The core is total**: no `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!`, `unreachable!`, `[i]` indexing or string slicing in any crate (use `get` and the `Option` combinators; a `debug_assert!` states an invariant the tests check). Clippy denies each of them on every target.
- Conversions through `From` and `TryFrom`; a validated value is built once by `parse` or `try_from` at the edge (§ 6).
- **Folds and sorting**: a fold may carry an owned accumulator (`|mut acc, x|`) with no marker. Sorting, grouping and joining go through `itertools` (allowed and preferred: `sorted`, `sorted_by_key`, `partition_map`, `join`, `chunk_by`), never a hand-written sort or grouping.
- **Dropped errors**: `.ok()` is allowed where absence is the meaning; an error that is dropped is logged first with `inspect_err`, then `.ok()`.
- **Typestates**: a state machine keeps its typestates (`EmailLogin<S>`, `Cached<Trust>`, `Memory<Trust>`, the socket's `Synced`, `Pending`, `Acked`). A marker is an empty enum, never a unit struct: it cannot be built. A state known only at run time (the wait for an ack, the load handshake) is a runtime enum; a `bool` beside an `Option` (`WritePlan`) becomes typed states.
- **Accepted exceptions**: `&mut TransactionMut` in the pure `change/apply.rs` (the library imposes it; it is in memory); `Client` holds `Arc<RwLock<State>>` as the one shared mutable state (the session machine stays pure); bounded recursion (`compact::prune`, JSON) names its bound in a `//` why-line; the async read loops marked `// imperative:` stay loops (no `try_unfold` or `try_fold`).
- **Pure is a property of a module**, not of a crate: the `PURE` list names modules, shells' included, and the layer check treats them so.
- **Validation accumulates**: independent checks return `CheckAll` and report every failure at once, not the first.
- Illegal states are unrepresentable: fields that can contradict each other merge into one enum. No `bool` + `Option` pairs, no `Option<Option<_>>`, no `Option<Vec<_>>` unless empty and absent differ.
- **Limits, checked by the conventions scan (an internal check, not shipped)** (not by clippy configuration, § 7): a function stays within 60 lines; a signature has at most 4 parameters, `self` included (a group passed together becomes a struct); nesting stays under 4 levels; a file stays within 800 lines.
- No magic values: numbers, strings that name a protocol field and glyphs are named constants owned by the code that uses them; tables are data, not match arms.
- Resources are RAII: raw mode, echo off, temporary files and locks are restored by a guard's `Drop`, with a `GUARD:` note (below).
- **Macros are minimal.** No `macro_rules!`, anywhere, tests included. A procedural macro lives in `holst-macros` and only where it removes real repetition; the doc comment of each macro states the repetition it removes (what, how many sites) and why a generic function or a table cannot. Anything that a generic function, one implementation for a pair, or a table can do is not a macro. Kept today: the endpoint attributes `get`, `post`, `patch`, `delete` (one declarative line per endpoint, about 60 endpoints, each also emitting its `ApiRequest` as data) and the `CommandArgs` derive (about 55 arguments structs: each makes its entry of the command table, the arguments both the command line and the MCP schema are built from, which a generic function cannot do at compile time). Removed: `derive(Through)`; a foreign error converts through thiserror's `#[from]` or a hand-written `From`.
- The public API is minimal: `pub(crate)` unless another crate needs it; a type has only the methods code calls now; no re-exports or shims for compatibility: delete, never deprecate. No production code that only tests call.
- **Files**: a file is written with std only, through `holst_core::fs`: write a sibling partial file, then `rename` over the target; a file holding a secret is created private (mode 0600) from the start. A partial file a crash leaves is recognised by `is_partial` and overwritten by the next save. No fsync unless a measurement asks for it.

**Comments** are few. The workspace has about 2,300 `///` and `//!` lines in total, the density of the reference project; a change that grows the total beyond the code's growth is a finding. The rules:
- A doc comment (`///`) goes on a public item whose name and signature do not say it all. Not on fields, enum variants whose name is the sentence, constants, private items, trait implementations, `new`, accessors, or an error variant (its `#[error]` text is its documentation).
- One line; the first word of a function's doc is a third-person verb (`Returns…`, `Loads…`), of a type's the noun (`A board…`). No restating of the type, no essay, no local path, no history.
- More than one line only for a required `# Errors` or `# Panics` section (kept: the lints `missing_errors_doc` and `missing_panics_doc` are denied; one line, `As [`X`].` when another item says it), for a doc example that is also a test, and for a macro's justification.
- A module has at most one `//!` line saying what it holds; the crate root of a library may add the one example.
- `//` inside code says why, one line, never what. Three notes are the only fixed forms: `SAFETY:` above an `unsafe` block (the invariant that makes it sound; `undocumented_unsafe_blocks` is denied), `PROTOCOL:` on a line that follows a rule of the Holst board protocol (names the section of `docs/board-protocol.md`), and `GUARD:` on a `Drop` impl (names what it restores). `// imperative:` is the fourth, from the first rules above.
- No commented-out code, no `TODO`, no section banners (`// ── … ──`).

## 4. Tests

- **One behaviour, one test**, named `<subject>_<behaviour>` after the behaviour, not after the function (`board_text_search_ignores_case`). A behaviour change edits its test in the same commit. A defect gets a failing test first; it passes ⇒ the report was wrong.
- **Pure core**: tested by value in and value out, no fakes: tables with `rstest`, laws and round trips with `proptest`, snapshots of rendered text with `insta` (committed; CI runs with `INSTA_UPDATE=no`).
- **Unit tests know the implementation, integration tests only the behaviour.** A unit test (`#[cfg(test)] mod tests` or the `tests.rs` beside a module) may call private items and use canned stand-ins: a canned `Transport`, `FixedClock`, `SequentialIds`, `RequestReporter` (a dry run). An integration test (`crates/*/tests/`, `crates/holst-scenarios`) sees only the public API or the built binary, a temporary home and the fake Holst (`holst-fake`: a stateful HTTP server on `127.0.0.1`); it names no internal type and asserts on what a user observes (output, exit code, the fake's state), never on request paths when a result shows the same. A real network or a real session is never reached from a test.
- **Shells** are tested through the binary against the fake Holst: one scenario runs on both surfaces (`#[values(Surface::Cli, Surface::Mcp)]`; `holst` by arguments, `holst-mcp` through the `rmcp` client), so the two cannot drift; interactive forms through a pseudo-terminal (`expectrl`, the `pty` module). Logic is tested in the core, the shells by one or two scenarios per command or tool. Each exit code of § 5 has a test.
- **Placement**: behaviour (BDD-style) tests of a crate's public contract live in the crate's `tests/`, named as behaviours (given / when / then in the test name or a doc line). Internal tests live in `#[cfg(test)] mod tests` at the bottom of the file they test (a `#[cfg(test)]` module ends its file), or in a `tests.rs` beside the module when the file would pass 800 lines, so no function exists only for tests. **One test binary per crate**: `tests/main.rs` with a module per area. Tests name items by `crate::` path, never `super::`, and import them by name, never by glob. Fixtures are files under `tests/fixtures/`. A refactor leaves snapshots unchanged except renamed identifiers; a behaviour change comes with a new snapshot and its reason.
- **A test has no branch**: no `if`, `else` or `match` in a test function or a property body (helpers, fakes and generators may branch). It builds its input, runs the subject once and asserts on the whole result: `assert_eq!` on a value, `assert!(matches!(value, Pattern))` on a variant; a test that would branch is two tests or a table (`rstest`). Panics by `assert!` are how a test fails and are allowed.
- Tests touch no real home: every test sets a temporary `HOME`, `HOLST_HOME` and `HOLST_CACHE_DIR`. Live writes happen only by hand, on a board named `zz-test-*`, deleted afterwards.
- Test code obeys the production lints: `unwrap` and `expect` are not used, nor `panic!` or indexing; a test returns `Result` and uses `?`, an assertion is `assert!`/`assert_eq!`. `#[expect(lint, reason = "…")]` appears in tests only; production code has none (a test that must panic to assert carries `#[expect(clippy::panic_in_result_fn, reason = "…")]`). Test structure is kept to the production bar (§ 3), except the limits on function length.
- Benchmarks (`benches/`) run only on request, never in a task gate; a `let mut` justified by speed cites one.

## 5. Errors and user-facing text

- Every crate that can fail has one boundary `Error`. An error of parsing one value is `<Type>Error`; its out-of-range case is `OutOfRange` with the value and the bound (`{ value, max }`, plus `min` when the floor is not zero).
- An answer is the past tense of its ask with `result: Result<T, E>`; the cause field of an error is `error` (or `cause`) with an explicit `#[source]`, never `source`.
- `thiserror` enums with `#[error]` text on every variant and `#[source]` chains; no `String` payloads except a field named `detail` that holds the cause's own words (a server's or the OS's), never shown raw, and at most one `Other { detail: String }` variant per error enum, no `Box<dyn Error>`, no `anyhow`, no `.context()`, no hand-written `impl Error`. `From` along a real crate or domain edge only (`#[from]` on a boundary variant; `#[derive(Through)]` for a two-step lift). No `let _ =` and no `unwrap_or_default` or `.ok()` that hides an error.
- IO failures travel as values to the shell, which reports them; never as a panic. A shell never prints in the middle of the core; output happens at the edge.
- **User-facing text**: every error the user sees is a plain sentence that names the cause in the user's terms and, where the user can act, what to try (the core's `Hint` and `Advice` carry the second part). Never an OS code, an `ErrorKind` name, third-party error text, a Rust error chain or a source position; a bare category ("timed out") is not enough. The text is built where the error is reported, from the glossary's words. An HTTP status number is not in the sentence; it is shown only under `--verbose`.
- **Streams**: the result, and machine-readable output (`-o json`), go to stdout; errors, hints, logs and progress go to stderr. `holst-mcp` writes only protocol messages to stdout. Only the output module prints (`print_stdout` and `print_stderr` are denied everywhere else). A reader that closes stdout (`| head`) is success. No secret is accepted in a flag or logged.
- **Exit codes**: each user-facing error class maps to a distinct process exit code, BSD `sysexits` numbers, chosen by one function from the error and covered by tests; a single `1` for everything is a defect. The shell exits with `ExitCode`, never `process::exit` (`exit` is denied).

| code | class | from |
|---|---|---|
| 0 | success, including a closed stdout and `--help` | |
| 64 | usage: a bad flag, argument, address or value (clap's own errors included); a name that fits several things | clap, `InputError`, `LookupMiss::Ambiguous` |
| 65 | data: input or a stored file that is malformed | `HttpError::Json`, `LocalDataError`, `LoginError::NotEmail` |
| 66 | no input, or not found: a workspace, board, folder, object, file, cached board | `LookupError`, `LocalDataError::NotCached` |
| 69 | unavailable: the server refused, answered nonsense, or is down | `HttpError`, `BoardSocketError` |
| 73 | an output file exists or cannot be made | `LocalDataError::AlreadyExists`, `FileWrite` |
| 74 | IO: local storage, terminal or write failed | `LocalDataError`, `Error::WriteStdout` |
| 75 | temporary failure: the user may retry (timeout, rate limit, 5xx, a cancelled login) | `HttpError`, `BoardSocketError` |
| 77 | permission: not signed in, access refused, or a write that needs `--yes` | `LoginError`, HTTP 401 and 403, `Unconfirmed` |
| 78 | config: a bad environment or setting | `InputError`, `LocalDataError::CacheOff` |
| 129, 130, 143 | hangup, interrupt, terminate | `signals` |

Every error enum has no `_` arm in `exit.rs`: a new variant breaks the build until it is classed. `holst auth status` prints its report and then exits with the code of what it found (77 without a login or with a rejected one, the HTTP error's code when the server cannot be asked). The values in `exit.rs` are the truth; the same constants build the `EXIT CODES` block of `holst --help`.
`holst-mcp` exits 0 on a clean shutdown and 1 when it cannot start; a failed tool call is a reply with `isError`, never an exit. The code table is the `EXIT CODES` block of the `holst --help` text.

## 6. Newtypes and units

- A newtype exists when the value has a rule (a range, an invariant: `Finite`, `Positive`), a unit (pixels, bytes, seconds) or an identity the server gave (`BoardId`). A bare `f64` or integer in geometry, layout or time is a defect.
- Parse, don't validate: a newtype is built once, at the edge, by the only function that can build it (`parse`, `try_from`), and trusted after. A raw `String` id past the edge is a defect.
- Every `as` cast sits in a named conversion: a unit conversion is a method on the newtype that owns the unit, a plain numeric cast a function named by its result (`index_from_count`). A bare `as` in any other place is a defect; the cast lints are denied.
- A time is `Duration`, `Timestamp` or a value read from `Clock`, never a bare number; a field's name says what a count counts.
- A closed set of values is an enum, never a `&str` matched by text; values that can be swapped silently (a width and a height, two ids) are a struct or newtypes.
- No type without a rule: no trait with one implementation, no wrapper that only forwards, no option nobody calls. Before writing a helper, use what the codebase, std or an installed dependency (`itertools`) already has; never add a dependency for a few lines.

## 7. Reviews and checks

- A review cites rules as `§n`, outputs one list of findings ranked H/M/L, each with `path:line`, the rule, the defect and the fix; a reviewer reads, never edits. Besides the rules it checks error paths and edge cases (an empty workspace, a board with no objects, a missing or half-written session file, an expired token, a server that answers 5xx or never answers, a closed stdout, an interrupt in the middle of a write) and the exit code of each.
- **Lints**: all lint configuration lives in `Cargo.toml` `[workspace.lints]` and nowhere else. No `clippy.toml`, no `.clippy.toml`, no crate-level `#![allow]`/`#![deny]`, no `#[allow]`. The set is the strictest: `clippy::all`, `pedantic`, `perf`, `style` and `cargo` denied, plus the denied restriction lints already listed there (panics, indexing, casts, `print_*`, `allow_attributes`, `dbg_macro`); a lint is relaxed only by editing that table, with a comment, never at a use. A `#[expect(lint, reason = "…")]` is allowed in test code only.
- **Lints added to the table**: every lint the reference project denies that holst-rs lacks, so the set is the strictest of both: `wildcard_enum_match_arm`, `single_match_else`, `option_if_let_else`, `shadow_unrelated`, `same_name_method`, `unused_result_ok`, `let_underscore_must_use`, `let_underscore_untyped`, and the rust groups `rust_2018_idioms`, `rust_2024_compatibility`, `future_incompatible`, `unused`. `as_conversions` is not among them: the named conversions of § 6 would need an `#[expect]` at every cast; the conventions scan checks `as` instead.
- **Strict lints per crate**: the lints listed above are not all in `Cargo.toml` yet; each crate turns them on as it is cleaned.
- **Scan rule for `let mut`**: the style scan (an internal check, not shipped) rejects a `let mut` with no `// imperative:` line.
- **Checks to build**: a layer check (which crate or module may use which). The spellcheck of docs and comments exists: `just spellcheck` (cargo-spellcheck, config in `.config/spellcheck.toml`, words in `spellcheck.dic`).
- **Checks by tool**: the internal check runner (not shipped) with `style`, `fmt`, `clippy`, `tests`, `doc`, `e2e`, `macros`, run by `just check` and the gates. the style scan enforces § 3's FP rules, the pure-module list, `macro_rules!`, string errors, `anyhow` and one-line comments; the conventions scan enforces the numeric limits (function length, parameter count, file length, nesting), the ban on glob imports, `super::`, `get_*`, `let _ =` and bare `as`, the names of § 1 and the `//!`/`///` line count; the glossary lint (every type maps to a term or an Excepted word, no _Avoid_ word is a name) joins them with the glossary.
- Checked in review only: names against the glossary, variant shapes (§ 2), the field rule, the reason in each `// imperative:` line.
- Format and lint run on `nightly-2026-05-22`; build, test and doc on Rust 1.96 from `rust-toolchain.toml`.

## Dropped from the reference style

- The Kernel, the Drivers and the Model/Message/Command/Transition vocabulary (§ 1 crates and suffixes `Message`, `Effect`, `Spec`, `Revision`; § 3 `&mut` only in `update`, the `Command` monoid, `spec` slices, a Driver's file rule, the wake's clock): holst-rs has no machine-and-runtime architecture. The counterpart is effects as data.
- The ban on `then`, `and`, `merge`, `chain` on `Command` and on the word `Subscription`: holst-rs has no such types (the clap `Command` is not a monoid).
- The provider seam for the named music server and its identifier formats (§ 1): its counterpart is the wire seam above.
- Transition tables (§ 4 "one row, one test", "not applicable is asserted", row ids checked both ways), sans-IO machine tests, record and replay, deterministic simulation, model-based tests, fuzzing the update function: holst-rs has no state machine to drive; the pure core is tested by value and by property.
- Hardware tests and the emulated Windows run: holst-rs has no hardware; Linux and Windows are cross-checked in containers only at the end of the style work.
- Cognitive complexity 15, the function length of 100 and file length of 1500 set in `clippy.toml` (§ 3): holst-rs uses no `clippy.toml`; its own, lower limits (60 lines, 4 parameters, 800 lines, nesting under 4) are checked by the conventions scan.
- The machine's `now` parameter, the `Generation` suffix and `&mut` only in `update` (§ 1, § 3): holst-rs has no machines with a pass time.
- Generated references (config keys, keys, command-line flags) compared with the code (§ 7): holst-rs has none; `--help` is generated by clap.
- `unwrap` and `expect` allowed in test code (§ 4): they need `clippy.toml` or a per-test `#[expect]`; tests return `Result` instead.
- Design documents and ADRs as destinations of the intro: holst-rs has only `board-protocol.md`; reasons for hard choices go into the doc comment of the item or the commit message.
