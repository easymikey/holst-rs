# holst-rs

An unofficial CLI and MCP server for [Holst](https://app.holst.so), written in Rust. It logs in as you and does what you do by hand in the web interface, over the same channels as the web client: the REST API for workspaces, folders, files and sharing, and the live board sync (Yjs over WebSocket) for the objects on a board. Neither is public: both were reconstructed from the web client's traffic and may change without notice. The project is not affiliated with Holst; use it within Holst's terms of service.

- `holst`: a CLI for the terminal and scripts.
- `holst-mcp`: an MCP server for AI agents, with 11 tools and short replies (`@N` aliases instead of UUIDs, lists as tables).

Each works on its own: install only the CLI, only the MCP server, or both. Installed together, they share one login.

The command layout (`holst board ls`, `holst auth login`, `-o json`) is inspired by GitHub's [`gh`](https://cli.github.com) 🍦.

## Why

Holst has no public API and no CLI: everything happens in the browser, one click at a time. holst-rs makes boards scriptable and lets AI agents work on them.

- Scripts: list, search, export and back up boards, add and edit objects from the shell.
- Agents: an assistant such as Claude Code reads a board and adds stickers, text and comments through MCP. Replies are short, so they cost the agent little context.
- Safe by default: every write has a dry run, and irreversible ones need `--yes`.

## Install

Homebrew (macOS and Linux):

```sh
brew install easymikey/holst/holst-cli easymikey/holst/holst-mcp
```

From crates.io:

```sh
cargo install holst-cli
cargo install holst-mcp
```

From git:

```sh
cargo install --git https://github.com/easymikey/holst-rs --locked holst-cli
cargo install --git https://github.com/easymikey/holst-rs --locked holst-mcp
```

Release archives for macOS, Linux and Windows are on the [releases page](https://github.com/easymikey/holst-rs/releases).

## CLI

Log in once, then work:

```sh
holst auth login                              # email, then the one-time code from the email
holst board ls -w easy                        # boards and folders as a tree
holst board show "Plans Q4"
holst board snap "Roadmap" --frame "Q3"       # PNG in ~/Downloads, prints the path
holst object add sticker "Roadmap" "Idea" --at 100,100 -c yellow4
holst search "release"
```

Every write accepts `-n, --dry-run`. Irreversible actions need `-y, --yes`; without it nothing is sent and the exit code is 77.

## MCP

```sh
holst-mcp setup
```

`setup` logs you in and connects the server to your MCP clients:

1. Asks for your email and the one-time code Holst sends to it. The login is stored in the system keychain, or in `$HOLST_HOME` if set. The code never passes through the agent.
2. Adds the server to Claude Code and Codex, if installed.
3. Prints config entries for other clients (Hermes, opencode, Cursor).

`--no-connect` skips steps 2–3, `--connect-only` skips step 1. A login from `holst auth login` works too.

If the login expires, the agent asks you to run `holst-mcp setup` again. The running server picks up the new login without a restart.

## Docker

**Server on the host, agent in a container.** Start the server over HTTP:

```sh
holst-mcp --http 127.0.0.1:8808 --print-config   # prints the client config with the token
holst-mcp --http 127.0.0.1:8808
```

In the container, connect the client to `http://host.docker.internal:8808/mcp` with the header `Authorization: Bearer <token>`. On Colima, `host.lima.internal` works too. The token is read from `HOLST_MCP_TOKEN` or from the `mcp-token` file next to the login. Log in on the host with `holst-mcp setup`.

**Server in the container.** Install the Linux binary from the release archive. The container has no keychain, so keep the login on a volume:

```sh
docker run -v holst:/holst -e HOLST_HOME=/holst …
docker exec -it <container> holst-mcp setup --no-connect
```

## Help

`holst -h` lists the groups, `holst <group> -h` the commands of a group, `holst help exit-codes` the exit codes.

## License

MIT OR Apache-2.0.
