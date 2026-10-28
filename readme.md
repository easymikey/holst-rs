# holst-rs

An unofficial CLI and MCP server for [Holst](https://app.holst.so), written in Rust. It works with your session token and does what you do by hand in the web interface. The API is closed: the endpoints were reconstructed from the web client's traffic and may change without notice. The project is not affiliated with Holst; use it within Holst's terms of service.

- `holst`: a CLI for the terminal and scripts.
- `holst-mcp`: an MCP server on stdio, with 11 tools and short replies (`@N` aliases instead of UUIDs, lists as tables).

## Why

Holst has no public API and no CLI: everything happens in the browser, one click at a time. holst-rs makes boards scriptable and lets AI agents work on them.

- Scripts: list, search, export and back up boards, add and edit objects from the shell.
- Agents: an assistant such as Claude Code reads a board and adds stickers, text and comments through MCP. Replies are short, so they cost the agent little context.
- Safe by default: every write has a dry run, and irreversible ones need `--yes`.

## Install

Homebrew (macOS and Linux):

```sh
brew tap easymikey/holst
brew install holst-cli holst-mcp
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

## First steps

```sh
holst auth login                              # one-time code from the email
holst board ls -w easy                        # boards and folders as a tree
holst board show "Plans Q4"
holst board snap "Roadmap" --frame "Q3"       # PNG in ~/Downloads, prints the path
holst object add sticker "Roadmap" "Idea" --at 100,100 -c yellow4
holst search "release"
```

Every write accepts `-n, --dry-run`. Irreversible actions need `-y, --yes`; without it nothing is sent and the exit code is 77.

## MCP

```sh
claude mcp add holst -s user -- holst-mcp
```

## Help

`holst -h` lists the groups, `holst <group> -h` the commands of a group, `holst help exit-codes` the exit codes.

## License

MIT OR Apache-2.0.
