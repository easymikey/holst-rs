# holst-rs

An unofficial command line and MCP server for [Holst](https://app.holst.so) boards, written in Rust. `holst` is for the terminal and scripts; `holst-mcp` gives AI agents such as Claude Code and Codex the same commands as 10 tools. Install either one; both use the same login.

Holst has no public API, so holst-rs logs in as you and uses the web client's own channels: the REST API and the live board sync. They were worked out from the web client's traffic and may change without notice. The project is not affiliated with Holst.

## Why

In Holst everything happens in the browser, one click at a time. holst-rs lets you script boards and lets an agent read and change them. The command layout (`holst board ls`, `holst auth login`, `-o json`) is inspired by GitHub's [`gh`](https://cli.github.com) 🍦.

## CLI

### Features

- **Small and fast**: one native binary of about 10 MB that starts in under 5 ms (`gh` is 40 MB and takes about 30 ms). Tests pin how many requests each command sends to Holst.
- **Boards and folders**: list them as a tree, create, rename, move, copy, tag and favorite them, send them to the trash and back.
- **Looking at a board**: `show` prints a board or one frame with its objects; `snap` draws it as PNG or SVG.
- **Objects**: add 22 kinds, from stickers, text, shapes and arrows to tables, kanbans, task cards, mind maps, code blocks and freehand strokes. Edit, copy to another board, group, lock, align and remove them.
- **Files**: export a board to a file and import it back, or save a board with its pictures to a folder.
- **Backups**: list, download and restore the server's backups of a board.
- **Comments**: read, start, reply to, resolve and delete comment threads.
- **Sharing**: give and take a user's right on a board, change who can open it, and hand it to a new owner.
- **Search**: find boards by name or by the text on them.
- **Workspaces and templates**: create workspaces, see members and the event log, and manage templates.
- **For scripts**: `-o json` gives the full data, and each kind of failure has its own exit code (`holst help exit-codes`).
- **Safe writes**: `-n` shows a change without sending it. Deleting for good and other changes that cannot be undone ask on the terminal and need `-y` elsewhere. A name with several matches is listed, never guessed.
- **Login**: the session lives in the system keychain and refreshes itself; `storage clean` removes old logins and stale copies.

### Install

```sh
brew install easymikey/holst/holst-cli       # macOS and Linux
cargo install --locked holst-cli              # from crates.io
```

Archives for macOS, Linux and Windows are on the [releases page](https://github.com/easymikey/holst-rs/releases) as `holst-cli-*`; the Linux ones are static and run on any distribution.

### Quick start

```sh
holst auth login                                 # email and a one-time code
holst board ls -w easy                           # boards and folders as a tree
holst board snap "Roadmap" --frame "Q3"          # PNG in ~/Downloads
holst object add sticker "Roadmap" "Idea" -c yellow4
```

`holst -h` lists the command groups, and `holst <group> <command> -h` shows the flags.

## MCP

### Features

- **Small and fast**: one native binary of about 11 MB that starts in under 5 ms, with no runtime to install.
- **10 tools**: one per command group, with the same boards, objects, comments, sharing, search, workspaces, templates and backups as the CLI.
- **Reads and writes apart**: changes to boards and objects live in `board_write` and `object_write`, so your client can allow reading once and ask before each change. Comments, templates, backups and workspaces have one tool each for reading and writing.
- **Safe writes**: `dry_run` shows a change without sending it. A change that cannot be undone needs `confirm`; without it the server replies with what it would do.
- **Short replies**: objects show as `@N` instead of UUIDs and lists as tables, so the agent spends little context. `output: json` gives the full data.
- **Login outside the agent**: you type the one-time code in your terminal, never through the agent, and the running server picks up a new login without a restart.
- **Over stdio or HTTP**: HTTP with a token serves agents in containers (see [Docker](#docker)).

### Install

```sh
brew install easymikey/holst/holst-mcp       # macOS and Linux
cargo install --locked holst-mcp              # from crates.io
```

Archives are on the [releases page](https://github.com/easymikey/holst-rs/releases) as `holst-mcp-*`.

### Quick start

```sh
holst-mcp setup
```

`setup` logs you in, adds the server to Claude Code and Codex if they are installed, and prints the config for other clients such as Cursor. Run it again when the login expires; the agent tells you when.

To add the server to Claude Code by hand:

```sh
claude mcp add --scope user holst-rs -- "$(command -v holst-mcp)"
```

## Docker

Agent in a container, server on the host: start the server over HTTP.

```sh
holst-mcp --http 127.0.0.1:8808 --print-config   # prints the client config and its token
holst-mcp --http 127.0.0.1:8808
```

The client connects to `http://host.docker.internal:8808/mcp` with that token.

Server in the container: copy `holst-mcp` from a Linux `holst-mcp-*` archive into the image (it is static, so Alpine or `scratch` works). A container has no keychain, so keep the login on a volume:

```sh
docker run -v holst:/holst -e HOLST_HOME=/holst …
docker exec -it <container> holst-mcp setup --no-connect
```

## For coding agents

Run `holst <group> <command> -h` for the flags of the installed version and `holst help exit-codes` for the exit codes. Pass `-o json` to read results.

Every write takes `-n, --dry-run`. Off a terminal, a change that cannot be undone needs `-y, --yes`; without it holst prints the dry run and exits with 77. Pass `--yes` only after the user agreed.

Never ask the user for the one-time code. Tell them to run `holst auth login` or `holst-mcp setup` themselves.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT or Apache-2.0, at your choice.
