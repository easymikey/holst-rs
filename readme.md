# holst-rs

[![Rust Version](https://badgen.net/badge/rustc/1.96%2B/orange)](https://www.rust-lang.org)

An unofficial CLI and MCP server for [Holst](https://app.holst.so), written in Rust. It
works with the user's session token and does what the user does by hand in the web
interface. The API is closed: the endpoints were reconstructed from the web client's
traffic and may change.

> [!WARNING]
> Unofficial client: the project is not affiliated with or endorsed by Holst. The API was
> reconstructed from the behaviour of the web client and may change without notice. Use it
> at your own risk and within Holst's terms of service.

## 💡 Motivation

- The official Holst MCP runs inside the app and offers 10 tools. This project is a
  standalone one: a CLI for scripts and the terminal and an MCP server for agents, both over REST.
- Saves model context: 11 tools, schemas under 9 KB.
- Replies are 4-8 times shorter: `@N` aliases instead of UUIDs, no `null` and no service
  fields, lists as tables.
- Fast board search: a compact index, ~12 µs per 10k nodes; trees are read in parallel
  and cached.
- Safety: the token goes only to `app.holst.so` over https, and irreversible actions need
  explicit confirmation.

## 📦 Crates

| Crate | Purpose |
|---|---|
| [`holst-core`](crates/holst-core) | token and login, REST client with retries, tree search, the command table and its execution |
| [`holst-cli`](crates/holst-cli) | the `holst` binary |
| [`holst-mcp`](crates/holst-mcp) | the `holst-mcp` binary: an MCP server on stdio |

## 📥 Install

Homebrew (macOS and Linux):

```sh
brew tap easymikey/holst
brew install holst-cli holst-mcp
```

Or in one step: `brew install easymikey/holst/holst-cli`.

macOS and Linux (x86_64, aarch64; Linux needs glibc 2.35 or newer). The script downloads the latest release, checks its sha256 and installs `holst` and `holst-mcp` into `~/.local/bin`:

```sh
curl -fsSL https://raw.githubusercontent.com/easymikey/holst-rs/main/install.sh | sh
```

Windows (x86_64), PowerShell:

```powershell
irm https://raw.githubusercontent.com/easymikey/holst-rs/main/install.ps1 | iex
```

`HOLST_VERSION=v0.1.0` picks a release, `HOLST_INSTALL_DIR` the target directory. Archives and `checksums.txt` are on the [Releases](https://github.com/easymikey/holst-rs/releases) page.

With Rust ([rustup.rs](https://rustup.rs)), straight from GitHub:

```sh
cargo install --git https://github.com/easymikey/holst-rs --locked holst-cli
cargo install --git https://github.com/easymikey/holst-rs --locked holst-mcp
```

From a clone (`scripts/install-from-source.sh` or `scripts/install-from-source.ps1` does the same and, on macOS, signs both with `HOLST_SIGN_IDENTITY`):

```sh
cargo install --path crates/holst-cli --locked
cargo install --path crates/holst-mcp --locked
```

Next: `holst auth login`, then connect the MCP server (`claude mcp add holst-rs -s user -- holst-mcp`).


## 🔑 Login

```sh
holst auth login          # code from the email; the form asks for the address and the code
holst auth login --email you@example.com
holst auth status        # where the login is stored and whether it is alive
holst auth logout
```

Login is by email only: the server sends a one-time code and the form asks for it with
hidden input (the screen shows the mask `▪`). If the account has no code login but has a
permanent password, the form asks for the password. SSO-only accounts are not supported.

A login is a pair: a token and cookies. The token goes into `Authorization: Bearer`, but
on its own it means nothing: it is a CSRF value, and the session lives in the cookies the
server issues (the web client sends requests with `withCredentials`). So the client
remembers `Set-Cookie` and returns the cookies with every request, and everything is
stored together.

The store is Keychain (macOS), Credential Manager (Windows) or Secret Service (Linux); if
it returns no entry, `~/.config/holst/token` with mode `0600` is used. If `HOLST_HOME` is
set (or `--home <dir>`), the login is kept only in `$HOLST_HOME/session` with mode `0600`
and Keychain is not touched: this is how tests, CI and a second profile next to the main
one work. No secret is read from environment variables. A personal `holst_pat_…` **does
not work**: its scope is `audit-events-read`, and on `/gapi/*` it gets 401.

Cookies the server refreshed during a command are saved to the place the login was read
from: a login from `~/.config/holst/token` stays in that file even if Secret Service is absent. If saving failed, the command still succeeds and prints `warning:` with the reason and a hint. A new `holst auth login` without Secret Service does not silently write to the file: it asks you to start gnome-keyring or KWallet or to set `HOLST_HOME`.

### macOS: Keychain access

On macOS `holst` and `holst-mcp` do not call Keychain themselves but run the system utility `/usr/bin/security` (as GitHub CLI does): the entry is bound to Apple-signed `security`, not to the hash of the build's code, so macOS no longer asks for access after a rebuild or an update. The secret is passed to `security` through stdin in hex and does not appear in the process arguments (`ps`). The price: any process of your user can read the entry without a prompt through `security find-generic-password -s holst-rs -w`.

Moving from older versions: `security` cannot read an entry created before this change without permission, so on the first run of the new version macOS asks once for access to "holst-rs" for `security`; click **Always Allow**. After that there are no prompts and you do not need to log in again.

If you do not want to keep the login in Keychain, keep it in a file: `export HOLST_HOME=~/.config/holst` in the shell profile and `-e HOLST_HOME=$HOME/.config/holst` for MCP, then run `holst auth login` once. The session lives in `$HOLST_HOME/session` with mode `0600`, Keychain is not touched. Signing the binaries for Keychain is no longer needed.

## ⌨️ CLI

Command form: `holst <group> <verb> <board> […]`. Groups: `auth`, `workspace` (`ws`), `board` (`b`), `object` (`o`), `template` (`tpl`), `comment`, `backup`, `storage`; at the top level `search` and `api`. There are no other aliases. The full list is `holst -h` and `holst <group> -h`.

### A board and a part of it

A board can be named by id, by a Holst link (`https://app.holst.so/share/b/<board>?objectId=<object>`, `/board/<id>?objectId=…`, `open-in-desktop?boardId=…&objectId=…`) or by part of its name; several matches are an error with a list, not a guess. A link to a frame or an object selects that part wherever a command takes a part of a board (`board show`, `snap`, `save`). A part of a board can also be given by flags: `--rectangle X,Y,W,H`, `--frame`, `--object` (repeatable), `--padding`.

```sh
holst board ls -w easy                       # boards and folders as a tree
holst board show "Plans Q4" --with members,sharing
holst board snap "Roadmap" --frame "Q3"      # PNG in ~/Downloads/<name>-<id>.png, prints the path
holst board snap "Roadmap" q3.svg --embed    # SVG; --embed puts the images inside
holst board save "Roadmap" --dir ~/tmp       # folder <name>-<id>/: picture, SVG, objects, original images
holst object ls "Roadmap" -k sticker --contains "design"
holst object add sticker "Roadmap" "Idea" --at 100,100 -c yellow4 -n
holst object edit "Roadmap" --object a1b2c3d4 --at 300,100
holst object rm "Roadmap" --object a1b2c3d4
holst object get "Roadmap" --object a1b2c3d4 # original of an image or file into ~/Downloads
holst search "consent"                       # board and folder names and text on boards, best first
```

`object add` takes the object kind as a word: `sticker`, `text`, `shape`, `frame`, `arrow`, `link`, `image`, `file`. Without `--at` the object goes to the first free place (inside the frame given by `--frame`, or, if a link to an object is given, inside its frame). `--items FILE` (a JSON array or JSON Lines, `-` is stdin) writes many objects in one write; the word `#N` in an item names an earlier item (the end of an arrow). `object edit` changes text, angle, size, colour, layer order and tags; `object cp` copies objects within a board or to another board.

### Other groups

```sh
holst board create "zz-test" -w easy
holst board rename|mv|cp|rm|restore <board> …   # rm goes to the trash; for good: --permanently -y
holst board favorite|unfavorite|touch|leave <board>
holst board grant|revoke|transfer|share <board> …
holst board export General general.ydoc      # the whole board document, gzip: general.ydoc.gz; --uncompressed, --object-list
holst board import general.ydoc.gz -w easy --name "Copy"   # a new board; --into board [--merge] -y writes into an existing one
holst workspace ls|show|create|rename|log
holst template ls|show|create|edit|rm
holst comment ls|add|reply|resolve|rm|read <board> …
holst backup ls <board>
holst backup get <key> snap.yjs             # gzip: snap.yjs.gz; --uncompressed
holst backup restore <board> <key> -y
holst storage ls                             # what holst keeps on disk and how much
holst storage clean -n                       # old login copies, data of earlier versions, temporary files
holst api GET /gapi/current_user             # anything not wrapped in a command
```

Files and folders are never overwritten: if the name is taken, it is an error, not a silent replacement; the default name is made from the board title and id, and characters unsafe for the file system are replaced. `-` instead of a file means stdout. `object get` for an object without a file is an error with an explanation.

PNG images are real: before rendering, files are downloaded in parallel (up to 6 at once, 20 s and 32 MB per image) only from the public storage `https://storage-edge.app.holst.so/storage/`, without token and cookies, and put into the `assets/` cache next to the boards (mode `0600`, up to a quarter of `HOLST_CACHE_MAX`, those unused for a month are deleted, `storage clean` and `auth logout` clean them together with the boards). resvg itself does not touch the network or files: it gets the images from memory. Without a network, with `--offline`, or on an error, a grey placeholder stays. SVG still links to the storage, `--embed` embeds the images. `object ls -o json` and MCP `object` with `output: json` show, for images and files, `file`: the name in the storage, the original name and the download address. The original is up to 200 MB; MCP saves it through `object_write` with `action: get` (not into `~/.ssh`, `~/.gnupg` or the login folder). PNG is built into `holst`; a `--no-default-features` build handles only SVG.

A board copy and a server snapshot are written as gzip by default (General: 4.6 MB → 1.4 MB), `board import` reads both gzip and a plain file; the object list next to it is written only with `--object-list`, JSON Lines without links.

Every write accepts `-n, --dry-run`: names are resolved by reading only, lines "would …" are printed (the same as dry run in MCP, with `dry_run`), and nothing is sent. `-y, --yes` is needed for what cannot be undone or touches other people's things: `--permanently`, a folder with boards for `board rm` and `board mv`, a frame with contents and several objects for `object rm`, rights and owner (`board grant`, `revoke`, `share`, `transfer`), `board import --into`, `template rm|edit`, `comment rm`, `backup restore`, `storage clean --all`, `workspace rename`, `holst api` with any method except GET. Without `-y` the request is not sent: a dry run is printed and the exit code is 77. One board to the trash and one object with `object rm` need no confirmation.

`holst storage ls` shows everything holst keeps on this machine: the login (Keychain, the file `~/.config/holst/token` or `$HOLST_HOME/session`), an old copy of the login that is no longer read, the browser profile left by the removed browser login (`~/.config/holst/browser`, with old cookies) and temporary files of an interrupted login write. `holst storage clean` deletes all of it except the current login; `--cache` also removes the board cache, `--all` the cache and the login (like `holst auth logout --local-only`: the session on the server stays, `holst auth logout` ends it). Symbolic links are neither deleted nor followed. `holst auth logout` hints if such leftovers exist.

Board cache: `board show|snap|save|export`, `object ls|get` and writes take the board document from the cache and always compare its state vector with the server (one frame of about 2 KB); the link to the full snapshot (4.6 MB for General) is downloaded only if the cache lacks something. A deletion in Yjs does not move the state vector by itself, so the cache is trusted for no longer than `HOLST_CACHE_TRUST` (default `24h`; `30m`, `7d`, a number is seconds; `0` means always download) after the last full download: after that the whole board is downloaded. A copy that is ahead of the server (the server was restored from a backup, `HOLST_BASE_URL` points to a copy with the same ids) is thrown away and the board is downloaded in full; a board that became inaccessible (403, 404, `forbidden`) is dropped from the cache. The MCP server keeps boards in memory and uses the same disk cache as the CLI. The cache lives in `~/Library/Caches/holst` (Linux: `$XDG_CACHE_HOME/holst` or `~/.cache/holst`, Windows: `%LOCALAPPDATA%\holst\cache`, with `HOLST_HOME`: `$HOLST_HOME/cache`) under the layout `v2/<host>/<user id>/`; `HOLST_CACHE_DIR` sets your own directory (absolute path only), `HOLST_CACHE=off` or `--no-cache` turn the cache off, `HOLST_CACHE_MAX` (256M by default) limits its size. Directories are `0700`, files `0600`; the cache holds other people's boards, so `holst auth logout` deletes the account's cache in all known cache directories and for all servers, even with `--no-cache` (if the account is unknown, the cache of all accounts), `holst storage ls` shows each account's cache as `cache`, `storage clean` removes from every account whatever exceeds the limit, whatever was unused for a month and old layouts (`v1`), and `--cache` removes the whole cache. Commands that read a board accept `--refresh` (download again) and `--offline` (no network, an error if the board is not in the cache); `--offline` reads the cache of the logged-in account: `holst auth login` saves the user id together with the session.

Output is `-o text|json|pretty`, logs go to stderr: `-v` or `HOLST_LOG=warn,holst_core=trace`. Notes (for example, a `board snap` scale at which the text cannot be read) go to stderr and start with `note:`, warnings start with `warning:`. Exit codes: `holst help exit-codes`.

The target of a change by name is chosen only by an exact match or a substring; several candidates are an error with a list. A whole name hides only partial matches of the same kind: a folder "Plans" next to a board "Plans Q4" is an ambiguity, and `board rm plans` asks for an id. A frame (`--frame`) is looked up as: exact id or link, exact title, an id prefix of at least 8 hex characters, a unique part of the title; if a prefix and a title match different frames, it is an error with a list by title. `/` as the destination folder is the root of the workspace. MCP `@N` aliases are accepted by the command neither in arguments nor in flags (`-w`, `--folder`, `--frame`, `--into`, `--object` and others). Invalid `HOLST_CACHE_MAX`, `HOLST_CACHE_TRUST` or `HOLST_CACHE_DIR` are not an error: one warning, and the command works without the cache, `auth logout` and `storage` too.

## 🤖 MCP

```sh
claude mcp add holst-rs -s user -- ~/.cargo/bin/holst-mcp
```

Eleven tools. A tool is a group of commands and `action` is the command's verb; the commands and arguments are the same as in the CLI, in snake_case (`dry_run` instead of `-n`, `confirm` instead of `-y`). Tools are split by risk so that the client asks permission only where it is needed: reading and writing are different tools.

| Tool | What it does |
|---|---|
| `auth` | login, account and workspaces; start here |
| `workspace` | `ls`, `show`, `log`, `create`, `rename` |
| `board` | reading: `ls`, `show`, `snap` (the picture right in the reply) |
| `board_write` | changes to boards, folders and rights: `save`, `export`, `import`, `create`, `edit`, `rename`, `mv`, `cp`, `rm`, `restore`, `favorite`, `unfavorite`, `touch`, `leave`, `grant`, `revoke`, `transfer`, `share` |
| `object` | reading the objects of a board or of a part of it |
| `object_write` | `add` and `edit` in batches (`items`), `cp`, `rm`, `get` (download the original of an image or file) |
| `template` | `ls`, `show`, `create`, `edit`, `rm` |
| `comment` | `ls`, `add`, `reply`, `resolve`, `rm`, `read` |
| `backup` | `ls`, `get`, `restore` |
| `search` | boards and folders by name and text on boards |
| `api` | any endpoint; non-GET only with `HOLST_MCP_RAW_WRITE=1` |

Replies carry `@N` aliases instead of UUIDs; they can be passed to any parameter, including the path and body of `api`; full ids are available with `output: json` on `board` and `object`. Aliases live as long as the process; the last 131,072 are remembered, and an older `@N` is an error asking you to take a fresh list.

## 🛠 Development

Build and tests run on 1.96.0 from `rust-toolchain.toml`; `fmt`, `clippy` and `doc` on
`nightly-2026-05-22` (`just nightly-install`). Fresh clone: `just setup` (`just tools`
+ `pre-commit install`).

```sh
just              # fmt + clippy + tests
just test [filter]
just check        # without tests
just matrix       # each feature separately
just coverage
```

Benchmarks: `cargo bench -p holst-core --bench pure`.

## ⚠️ Limitations

- Objects on a board live in Yjs over `wss://app.holst.so/hud/ws`: stickers, texts, shapes, frames, arrows, links, images and files can be read (`object ls`) and written (`object add|edit|rm`, MCP `object_write`); the other types (tables, cards and others) are read-only. Protocol: `docs/board-protocol.md`.
- Verified live on 22.10.2026: login by the emailed code, `status`, `whoami`,
  `ws list/tree/trash/users/activity/favorites/recent`, `board get/users/permits`,
  `template categories/personal`, `comment threads`, `backup list/download`,
  and for MCP `status`, `board action=ls`, `get what=board|backups`, `search`.
  The rest is written from captured traffic, and the tests use mocks. To verify:
  - creating a board in the root and in a folder (`parent_type=channel`), `order_index`;
  - the multipart field name of a file upload.
- The session token grants all the owner's rights: do not commit or share it. Experiment on
  other people's boards only with the owner's consent.

## 📄 License

Your choice of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
