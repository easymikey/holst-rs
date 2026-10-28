# Architecture

This file is the map: what holst-rs is made of, where each thing lives, and the rules that are true everywhere. It changes rarely. The words are defined in [GLOSSARY.md](GLOSSARY.md); the code rules are in [docs/conventions.md](docs/conventions.md); how the Holst board protocol works is in [docs/board-protocol.md](docs/board-protocol.md); what the tools do is in the [readme](readme.md).

The code is being moved to this layout. Until the move is done, `holst-core/src` is flat. A module whose name waits for a glossary term is marked `(term)`. This paragraph goes when the move ends.

## Bird's-eye view

holst-rs is an unofficial client for Holst (app.holst.so), a whiteboard service. It reads and changes workspaces, folders and boards over the Holst REST API, and the content of a board (its Yjs document) over the board socket `/hud/ws`. Two programs share one library: `holst`, a command line, and `holst-mcp`, an MCP server on stdio for language models. Both use the login that `holst auth login` saves. Both are built from one command table in the core: a command is a group and a verb (`object add`, `board snap`) with a risk (read, write or destroy) and its arguments, and the table makes the clap tree of `holst`, the tools of `holst-mcp` and the text of both helps.

The library is a pure core with effects at the edge. Pure code takes values and returns values: a board's objects, a change to them as one Yjs update, a picture of an area as SVG, the match for a name typed by a user. What must happen outside is computed first and returned as data: an `ApiRequest`, a cache `Plan`, the plan of a board write. An interpreter at the edge carries it out: sends it, or only reports it for `--dry-run`. Only the interpreters touch the network, the disk, the clock or randomness, and each sits behind a trait a test replaces.

```text
 argv, env ─► holst (clap) ─┐                         ┌─► stdout / stderr, exit code
 JSON-RPC on stdin ─► holst-mcp (rmcp) ─┤             └─► tool result on stdout
                                        ▼                         ▲
                         command table + use cases ───────────────┘
                               │            │
                 domain + protocol (pure) ◄── effect: requests, plans, ports
                               ▲            │
                               └── remote · storage · system (interpreters) ─► Holst REST, /hud/ws, disk, keychain, clock
```

## Code map

Folders are `crates/<name>`. A layer of `holst-core` is a top-level module of the same name.

**`holst-core`** — the library. It is the only crate that speaks to Holst, and the only one that knows its wire words.

- `error` — the thiserror enums, one file per domain (`HttpError`, `SocketError`, `LocalDataError`, …), the boundary `Error` and `Result`, and the advice under an error (`Hint`, in the words of the surface that shows it). `CheckAll` accumulates independent checks.
- `domain` — pure values, rules and plans: ids and addresses, text and time, geometry, layout and the palette, the workspace tree and its name index, lookups by id, name or link, sessions and logins, a board's objects, changes to them (build, check, write, the deletion journal), areas and pictures, board text indexes and search, the board file format. The request bodies of channels, sharing and uploads are plain values here, with their serde shapes in `protocol`. Replies the shells show as they came (templates, categories, backups, uploads, sharing) are domain values with serde, like `domain/node.rs`, and `domain/response.rs` holds `OtherFields`, `UnreadBody` and `Uploaded`; a template is its own wire shape and goes out as it came in.
- `protocol` — the Holst wire format that is not a value of its own: the frames of the board socket (lib0), the reply fields and the key tables the next step relies on (`protocol/json.rs`), the Yjs field names of a board's document (`protocol/document.rs`) and the serde shapes of the request bodies (`protocol/{channel,sharing,upload}.rs`). The module is `pub(crate)`, so the compiler keeps the shells out; its one public door is `protocol::json`, whose key tables the MCP reply compaction calls.
- `effect` — effects as data and the ports: `ApiRequest`, `HttpRequest` and `HttpResponse`, the traits `Transport`, `Clock`, `Socket`, `Connector`, the cache `Port`, the interpreter traits (`RequestInterpreter`, `WriteInterpreter`), the cache `Plan` and the board write plan, and the run's `Settings` (where the outside is).
- `remote` — the interpreters that talk to Holst: the reqwest transport, the `Client` (login headers, retries, redirects, reply status), the REST endpoints in `remote/api/<entity>.rs` (one `#[get]`/`#[post]`/`#[patch]`/`#[delete]` line each), the board socket over tokio-tungstenite with its proxy tunnel, loading and writing.
- `storage` — the interpreters of this machine's disk: files written whole or not at all (`storage/file.rs`), the size-capped board cache, the session store (keychain with a 0600 file, the macOS Keychain through `/usr/bin/security`), and the inventory of everything holst keeps.
- `system` — the rest of the machine: the system clock, random ids, PNG rasterising with the system fonts, the log subscriber.
- `command` — the command table and the use cases that compose ports. `command/table.rs` holds the table: each command's group, verb, risk, help and arguments, and the `CommandArgs` trait that turns the arguments of a call (one JSON object, from either surface) into the typed arguments struct of the command. A folder per group (`board`, `object`, `workspace`, `template`, `comment`, `backup`, `storage`, `auth`) holds the group's verb enum, its arguments structs (suffix `Args`) and the use cases: a board loaded, changed or forgotten through the cache, export and import of a whole board, the email login, lookups that fetch, assets.
- `laws` — the property laws of the pure functions; tests only.

**`holst-cli`** — the `holst` binary. `main.rs` is the composition root: it counts Ctrl-C first, parses, builds the run's settings from flags else the environment, runs one command and turns the result into an exit code (`exit.rs`). `table.rs` turns the core's command table into the clap tree and the clap matches back into one JSON object of arguments; `cli.rs` holds the global flags and the parse; `help.rs` prints the help in the style of `gh`. `commands/<group>/` holds what a group's commands print: `commands::run` hands a call to the group by its verb. `run.rs` gives a command its signed-in client; `args/` holds the settings flags. `output/` prints results to stdout and notes to stderr; `output/view.rs` turns values into text. `terminal/` restores the tty and handles signals; `form/` is the login form.

**`holst-mcp`** — the MCP server. `main.rs` answers `--version` and `--help`, else serves on stdio. `server/` is the `ServerHandler`; `server/table.rs` makes the tools from the command table: one tool per group, or two when a group has both reads and changes (`board` and `board_write`, `object` and `object_write`), each command an `action`; `tools/` holds the handlers by group and their pure argument checks; `login/` is the server's login as a pure state machine with its store; `memory/` keeps workspaces, trees, boards and the `@N` aliases between calls; `reply/` makes the text and images the model reads, compacted to save tokens.

**`holst-macros`** — procedural macros only: the endpoint attributes `get`, `post`, `patch`, `delete` and the `CommandArgs` derive, which makes a command's table entry from its arguments struct.

## Layers

```text
layer 5   holst-cli, holst-mcp ──► holst_core::{command, remote, storage, system, effect, domain, error}   never protocol (but `protocol::json`)
layer 4   command ──► remote, storage, system, effect, protocol, domain, error
layer 3   remote ──► effect, protocol, domain, error        storage ──► effect, protocol, domain, error        system ──► effect, domain, error
layer 2   effect ──► protocol, domain, error
layer 1   protocol ──► domain, error                        domain ──► error (and the Yjs field names of protocol::document)
layer 0   error ──► std, thiserror, and the error types of the crates it wraps
```

Every edge points down. `lib.rs` re-exports only `Error` and `Result`; `Client` and `Settings` are imported by path (`remote::client`, `effect`). `remote`, `storage` and `system` do not use each other: what needs two of them is a `command` module. A layer may use its own modules freely. A test module (`tests.rs`, `tests/`, `fake.rs`, `laws/`) follows its parent's row and may also use the fakes. The table a script checks is in the module-tree report, § Dependency rules.

The rules inside a shell:

- `main.rs` is the only file that reads the process environment (`std::env`) or builds an adapter of `system` or `storage`; everything else gets them through `Settings` and the environment it is handed.
- A pure shell module (`output/view.rs`, `error.rs`, `args/flag.rs`, `form/draw.rs`, `form/line.rs`; in `holst-mcp` `reply/`, `tools/input.rs`, `login/machine.rs`, `error.rs`) uses only `domain`, `effect` values and `error`.
- A command module does not use another command module; what two share is in `args/`, `run.rs` or the core (the `-y` rule is `Confirmation` in the command table). A tool module does not touch the login store: it asks `login/` for a connection.

What never lives in a shell: a Holst path, JSON key or socket message kind; a retry, redirect or timeout of a request; a cache decision; a check of a board change; a file write outside `storage::file`. A shell parses its input, calls the core, renders the result for its surface and chooses the exit.

## Data flow

**A CLI command**, `holst object add sticker <board> "hi"`:

1. `main.rs` registers the signal count, `Cli::parse_from_args` (clap, built from the table) parses, `args/settings.rs` builds `Settings` from flags, else a lookup into the environment.
2. `table::call` turns the matches into the table's `CommandName` and one JSON object of arguments; `commands::run` hands them to `commands/object/`; `run.rs` loads the saved login from `storage::session_store` and builds a `remote::client::Client`.
3. The core's `command/object` parses the object with `CommandArgs` into the arguments struct, turns it into a new object (`domain/change/value.rs`) and calls `command::board_load` to change the board.
4. The command loads the board's document from the cache, else over the socket (`remote/socket/load.rs`); `domain/change` builds, checks and writes the change as one Yjs update; `effect/board_write_plan.rs` decides what to send.
5. The `WriteInterpreter` carries it out: `WriteSender` sends it and waits for the server's ack; under `--dry-run`, `WriteReporter` only reports its size. The cache stores the new state after the ack.
6. `output/` prints each new object's id and link; `main.rs` returns `ExitCode::SUCCESS`.

**An MCP tool call**, `object_write` with `action: add` and `kind: sticker`:

1. rmcp reads the JSON-RPC request on stdin; the `ServerHandler` in `server/` finds the command in the table by the tool and its `action`, and its arguments are the same JSON object the command line makes.
2. `tools/input.rs` checks the arguments (pure); `memory/` expands `@N` and names into ids; `login/` hands out the connection, deciding with `login/machine.rs` and reading the store on the blocking pool.
3. `tools/board_load.rs` takes the board from `memory/board_memory.rs`, else the disk cache, else the server; from there the core path is the CLI's, steps 4 and 5.
4. `reply/` renders one line per new object, with its `@N` and link, and returns a `CallToolResult`.

## Errors and exits

The core returns `holst_core::Result`; each failure is a variant of a domain enum under `error/` with its cause as `source()`, never a string, never a panic. What the user should do next is a `Hint`, chosen in the core and worded for the surface. No layer below a shell prints anything or picks an exit.

In `holst`, a command returns `holst_cli::Error`, which wraps the core's (`Error::Holst`) and adds the CLI's own refusals. `exit.rs` is the one place an error becomes an exit code, and `exit_code` is the one `match` that does it, by the error's class in BSD `sysexits` numbers (a test in `exit.rs` pins each): a closed stdout (`holst … | head`) is 0; an error the login form already showed (`Error::Shown`) exits with the code of the error inside it; an interrupt is 130 (143 for SIGTERM, 129 for SIGHUP); 64 is a bad argument or flag, clap's usage errors included, or a name that fits several things (`EX_USAGE`; `--help` and `--version` stay 0); 65 is bad input data (`EX_DATAERR`); 66 is a named thing that does not exist (`EX_NOINPUT`: a board that is not found, a missing file, HTTP 404); 69 is a server that refused or is down (`EX_UNAVAILABLE`); 73 is an output file that exists or cannot be made (`EX_CANTCREAT`); 74 is a read or write that failed (`EX_IOERR`); 75 is a failure that a retry may cure (`EX_TEMPFAIL`: timeouts, HTTP 5xx and 429, a cancelled login); 77 is no login, no access, or a write that needs `--yes` (`EX_NOPERM`: HTTP 401 and 403); 78 is a bad environment or setting (`EX_CONFIG`). The core's error enums are exhaustive, so a new variant breaks the build until `exit.rs` classes it; the same constants print the `EXIT CODES` block of `holst --help`. An error that nothing showed yet is first reported on stderr through `output/view.rs`. The pattern is jj's: errors carry a kind and one function maps kinds to the exit (`jj/cli/src/command_error.rs`, `CommandErrorKind`), as uv's `ExitStatus` (`uv/crates/uv-command-support/src/lib.rs`) and ripgrep's `main` returning `ExitCode` (`ripgrep/crates/core/main.rs`).

In `holst-mcp`, a start failure (`StartError` in `main.rs`) is printed to stderr and exits 1 before the protocol starts. Once serving, every tool failure becomes a tool error, `CallToolResult::error` in `reply/`, so the model reads it: clients hide protocol errors. `ErrorData` is left to rmcp's own routing (an unknown tool, parameters that do not parse). A panic in a tool unwinds and kills only its request (no `panic = "abort"`).

## Borrowed patterns

- **One command table, a module per group**: holst's `command/table.rs`. The models are cargo, one file per command with its arguments and its run, `cargo/src/bin/cargo/commands/build.rs` (`cli()` beside `exec()`); a command group as a folder: jj, `jj/cli/src/commands/` (`bookmark/`, `git/`, `operation/`).
- **Commands call library operations, not the transport**: cargo, `cargo/src/bin/cargo/commands/build.rs` calling `ops::compile` in `cargo/src/ops/`; holst's `command` layer.
- **A thin `main` and one runner that owns the exit**: jj, `jj/cli/src/main.rs` (`CliRunner::init().run()`) and `jj/cli/src/cli_util.rs`; uv, `uv/crates/uv/src/lib.rs` (`run(cli) -> Result<ExitStatus>`).
- **The signed-in environment a command receives**: jj's `CommandHelper` in `jj/cli/src/cli_util.rs`; holst's `run.rs`.
- **Raw flags resolved into settings once, before any work**: ripgrep, `ripgrep/crates/core/flags/lowargs.rs` → `hiargs.rs`; uv keeps settings in a crate of their own, `uv/crates/uv-settings/src/`.
- **Output apart from commands**: jj, `jj/cli/src/ui.rs` and `formatter.rs`; uv's `Printer`, `uv/crates/uv-command-support/src/printer.rs`.
- **Tools on the handler, routed by macro, served on stdio**: rmcp, `rust-sdk/examples/servers/src/common/counter.rs` (`#[tool_router]`, `#[tool]`, `#[tool_handler] impl ServerHandler`) and `rust-sdk/examples/servers/src/counter_stdio.rs` (`serve(stdio())`).
- **Layers as a table of downward edges, and this file's form**: the architecture document of a sibling project, § Layers.

## Cross-cutting concerns

**Purity.** The pure modules are listed in `PURE` in the style scan (an internal check, not shipped); after the move a whole layer is pure (`error`, `domain`, `protocol`, `effect`). They may name the types of reqwest, tungstenite, keyring or resvg (an error's cause, a URL) but never call them.

**Wire words.** Holst's paths, JSON keys, request-body shapes and socket message kinds are in `protocol/` and `remote/`, the Yjs field names of a board's document in `protocol/document.rs` (each marked `PROTOCOL:`), which `domain/board` and `domain/change` import. The shells name glossary terms only.

**Files and secrets.** A file is written whole through `storage::file`: a sibling partial file, then `rename`; a file holding a secret is created 0600. A secret is a `secrecy` type: no value in `Debug`, memory zeroed.

**Testing.** Pure code is tested by value and by laws (`laws/`); an interpreter through its port with a fake (`remote/client/fake.rs`, `remote/socket/fake.rs`, `storage/cache/memory.rs`, all core-internal); a shell end to end on a scripted network (`commands/fake.rs`, `tools/tests.rs`) and in a pseudo-terminal (`crates/holst-cli/tests/pty.rs`). No test writes to a live board but `zz-test-*`.

**Later crate split.** `holst-core` stays one crate. `error`, `domain`, `protocol` and `effect` become a `holst-domain` crate only when the layer check has been clean for a while and either a consumer wants the pure layers without tokio and reqwest, or `cargo build --timings` shows `holst-core` on the critical path (criteria and costs: the module-tree report § 3).

**Size.** A file stays within 800 lines; a module's tests go to a sibling `tests.rs` when the file would pass it.
