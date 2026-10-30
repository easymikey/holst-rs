# Changelog

## 0.3.0

### Breaking

Flags and MCP keys renamed (the MCP key is the flag name with underscores):

- `board snap --format` is now `--picture-format` (`picture_format`)
- `object cp --to` is now `--into` (`into`)
- `object align --to` is now `--side` (`side`)
- `--size WxH` is now `--size w,h`
- `object edit --stroke` is now `--stroke-color`
- `object edit --text-bg` is now `--text-background`
- `object add link --no-embed` is now `--no-preview`
- `--uncompressed` is now `--no-compress` (`backup get`, `board export`)
- `-q, --query` is now `-p, --param` (`workspace log`)
- `template create --name <name>` is now positional `<name>`
- `board import <file>` is now `<path>` (the MCP key `file` is now `path`)
- `board grant --right <right>` is now positional `<right>`
- `template edit --body <body>` is now positional `<body>`
- MCP `mark` is now an array of values, not a comma string
- Kind words are kebab-case on the command line and in MCP (`sticker-stack`)

Removed:

- `holst api` and the MCP `api` tool. The MCP server has 10 tools.
- The `HOLST_MCP_RAW_WRITE` environment variable.
- MCP `refresh` and `offline` arguments. They stay on the command line.

Changed output:

- Board writes report one `status`: `sent`, `dry_run` or `held`, not `dry_run` and `accepted`.
- `object ls` JSON names the board key `board`, not `id`.
- Change reports show `@N` and the short id.
- A saved login records its base URL. A different `HOLST_BASE_URL` is refused.
- A socket internal error exits 75 (a retry may help).
- `object ls` lines show the corner as `x,y` and drop the kind padding, so MCP replies are shorter.

Releases:

- Each release has two archives per target: `holst-cli-<target>` and `holst-mcp-<target>`.
- Linux archives are static musl builds (`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`). The `-linux-gnu` archives are gone.

### Added

Object kinds and edits, by kind:

- Table: `object add table` with `--rows` and `--columns`. Edit cell text, insert and delete rows and columns, merge cells, split them again, clear, set a cell color.
- Document: `object add document`, a card of rich text with a color.
- Mind map: `object add mind-map` adds a root or a child node (`--parent`, `--direction`). `object edit --hide-children` and `--show-children`.
- Flip card: `object add flipcard` with `--back`. `object edit --flip` turns it, `--back` rewrites the back.
- Sticker stack: `object add sticker-stack` with a title and one of two widths.
- Dice and wheel: `object add dice` (`--faces`) and `object add wheel` (`--choice`, 2 to 20). `object edit --roll` rolls or spins, `--faces` changes a die.
- Stamp: `object add stamp` makes a text stamp.
- Reaction stamp: `object add reaction-stamp`, stuck to an object with `--on`. `object ls` shows the reactions on each object.
- Code: `object add code` with a language and a theme. `object edit --language` and `--theme`.
- Icon: `object add icon` makes a Phosphor icon with a weight, a color and a size. An unknown name is refused with close names.
- Sticker: `object edit --text-size`, `--show-author`, `--hide-author` and `--reaction`.
- Task: `object add task` builds a task card. `object edit` sets `--status`, `--start`, `--due`, `--description`, `--tag` and `--assignee` (by email, name or id).
- Kanban: `object add kanban` with columns and swimlanes. `object add task --kanban` and `object edit --kanban`, `--column`, `--swimlane` put a card in a cell or move it out.
- Drawing: `object add drawing` draws a stroke from `--point x,y` pairs. `object edit` sets its width (1 to 30) and color.
- Image: `object edit --name`, `--path` (replace the file), `--crop`, `--flip-image`, an outline and round corners.
- File: `object edit --display`, `--start-page`, `--hide-details`, `--show-details`, and `--extract` to put the pages of a file on the board.
- Text and color: `object edit --text`, `--text-format` and `-c` reach all the new kinds.
- `object ls` lists the details of each new kind. `media` holds the look of images, files and drawings in JSON.

Other:

- Terminal prompts. On a terminal, a held change asks to confirm, and an ambiguous name or a missing workspace opens a list to pick from. `holst-mcp setup` asks which clients to connect in one multi-select. `HOLST_NO_PROMPT=1` turns prompts off. Without a terminal and without `--yes`, a held change exits 77.
- `holst-mcp setup --clients claude,codex` connects the named clients without a question.
- `holst board tag` renames, recolors or removes a tag of a board. A removal is held without `-y`.
- `holst comment rm --reply` deletes one of your replies. `comment edit` colors and moves a thread.
- Boards of schema version 3 (`objects_v3`) are read and written.
- `holst-form` crate: the terminal prompts shared by the command line and `holst-mcp setup`.
- `holst-eval` (in `tasks/scenarios`): a harness that runs 20 tasks against the MCP tools.
- `scripts/release.zig` builds the release archives on one Mac when the GitHub runners are down.

### Changed

- The account's workspace list and board trees are kept on disk until logout. A miss, a 403, a 404, `ls` and `--refresh` read them again. Commands send fewer requests: a write reads its board once, and `board show`, `object add` and `object cp` reuse the reply of their load.
- `keyring` is version 4 on Windows and Linux/BSD (`secret-service` 5, `zbus` 5). Saved logins keep their service and account names.
- Linux builds in CI and in `release.zig` go through `cargo-zigbuild` 0.24.0 and Zig 0.17.0.
- `board show --with sharing` copies only the sharing keys.
- `holst storage` warns when the saved login cannot be read.
- File sizes are shown the same way on the command line and in MCP.
- One paging line ends `board ls` and `workspace log`.
- CI and the local gates run tests through `cargo-nextest`, with one retry policy for loopback stalls.
- `--tag none` clears the tags like `''`. `comment read --time` accepts ISO 8601 or says why not.
- A new object no longer copies the board, and `object edit --path` and `--extract` read the board once.
- `board snap` draws table grids with merged blocks, kanban columns with their cards, and mind-map connectors.

### Fixed

- A new group got the wrong corner, and JPEG sizes were read wrong.
- A call is repeated after a 401 only when no request of it landed.
- `logout` clears only the stores that still hold the login it ended.
- The MCP server keeps a good login across a re-login and a locked store.
- `holst-mcp setup` ends the replaced login and restores the terminal echo on every signal.
- `--http` keeps the login folder private and checks the `Origin` header and the token length.
- A failed channel read is an error, not permission to remove.
- Removing one group or kanban with content is held like a frame.
- A dry run of a cover reset checks your right. A failed note after a changed cover says so.
- `object add` checks named frames before any upload.
- A frame moved into a frame writes `parentId` only, no `zIndex`.
- Table ids are numbers, and each table edit is checked against the board it edits.
- Kanbans, task cards and merged table cells made by holst show in the web client: store entries carry their `id`, and merged blocks have numeric rows and columns.
- `object edit --extract` counts the pages of a new file first, and the page service gets the request shape the web sends.
- `object ls` prints a kanban card's stored width, and `--frame` lists objects nested in the frame's children.
- A card taken out of a kanban keeps the kanban's parent. A new mind-map child counts only the children on its side.
- A new start or due day keeps the times and the time zone of the task. A flip or a card move is refused beside `--at`, `--rotate`, `--size` and `--scale`.
- A kept board tree is read again on a miss and after a sent change, never after a dry run, and an older tree no longer overwrites a newer one.
- MCP write actions that hold nothing accept `confirm`, and a mixed-up key names the right one.

## 0.2.0

See the [0.2.0 release](https://github.com/easymikey/holst-rs/releases/tag/v0.2.0).

## 0.1.0

See the [0.1.0 release](https://github.com/easymikey/holst-rs/releases/tag/v0.1.0).
