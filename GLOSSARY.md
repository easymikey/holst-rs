# holst-rs

An unofficial command line and MCP server for Holst, the online whiteboard: the Account it logs in as, the Workspaces and Boards it reads and changes, the Objects on a Board, the Pictures and files it makes of them, what it keeps on this machine, and how it reaches the Holst server.

## Language

### Account and login

**Account**:
The Holst User holst acts for: their user id, Email address, name and Workspaces.
_Avoid_: current user, me, profile, whoami (except the command name)

**User**:
Any person with a Holst account, known by their user id; the one holst acts for is the Account.
_Avoid_: person

**Account id**:
The user id of the Account.
_Avoid_: user (alone)

**Organization**:
A group of Users in Holst with its own Templates, apart from any Workspace.
_Avoid_: org (except the server's), company

**Email address**:
The address a person logs in with.
_Avoid_: mail, login name, username

**Login**:
Logging in to an Account by Email address, with a Login code or a Password.
_Avoid_: sign-in, auth, authentication, credentials, flow, link

**Login methods**:
The ways the server offers an Email address to log in: a Login code, a Password, or none.
_Avoid_: options, offer

**Login code**:
The one-time code the server mails for a Login.
_Avoid_: temporary password, OTP, PIN

**Password**:
A permanent password the server asks for instead of a Login code.
_Avoid_: passphrase

**Secret**:
A value holst never shows or logs: a Login code, a Password, or a Session's token and cookies.
_Avoid_: credential, sensitive value

**Session**:
The token and cookies that sign every request, together with the Account id.
_Avoid_: token (alone), bearer, auth, credentials, login (for the value)

**Session state**:
Where the Session stands: missing, active or revoked.
_Avoid_: alive, rejected

**Session check**:
What the server says of the Session when asked: active, revoked or unknown.
_Avoid_: ping, alive

**Cookie jar**:
The cookies a Session carries.
_Avoid_: jar (alone)

**Rotated session**:
A Session whose cookies the server replaced during a Run, to be saved again.
_Avoid_: fresh session, persisted

**Server session**:
The Session as the server holds it, which a Logout ends.
_Avoid_: remote session, ending

**Saved login**:
A Session as the Session store keeps it between Runs.
_Avoid_: stored login, stored session, token file

**Session store**:
Where the Saved login lives: the Keychain or the Session file.
_Avoid_: store (alone), vault, credential store

**Keychain**:
The system's secret store: the macOS Keychain, the Windows Credential Manager or the Linux Secret Service.
_Avoid_: keyring, verdict

**Session file**:
The file that holds the Saved login when the Keychain is not used.
_Avoid_: token file

**Logout**:
Ending the Server session and forgetting the Saved login and the Account's Board cache.
_Avoid_: sign-out

### Workspace and tree

**Workspace**:
A shared space of Boards and Folders with its Members.
_Avoid_: space, team

**Workspace tree**:
The Boards and Folders of a Workspace in Sidebar order.
_Avoid_: sidebar, hierarchy, spec

**Tree node**:
One entry of a Workspace tree: a Board, a Folder, or another kind the server lists.
_Avoid_: channel, item, entry (alone)

**Board**:
One whiteboard in a Workspace, with Objects on it.
_Avoid_: canvas, sheet, page, channel

**Folder**:
A Tree node that holds Boards and other Folders.
_Avoid_: directory (alone), group, channel

**Parent**:
Where a Board or a Folder sits: the Workspace root or a Folder.
_Avoid_: destination, container, location, parent type

**Sidebar order**:
Where a Tree node sits among its siblings.
_Avoid_: order index

**Trash**:
Where deleted Boards and Folders wait to be restored or deleted for good.
_Avoid_: bin, recycle bin

**Deletion**:
How a Board or a Folder goes: into the Trash, or for good; a Folder takes its Boards along.
_Avoid_: removal, erasure

**Copy**:
A duplicate of a Board that the server makes, with its own name and Parent.
_Avoid_: duplication, clone, duplicate (except the command name)

**Template**:
A Board ready to start from, in the Account's own list, an Organization's, or a Template category.
_Avoid_: preset, blueprint

**Template category**:
A group of ready-made Templates, named by its slug.
_Avoid_: category (alone), catalog, section

**Member**:
A User with a Right on a Board, or a Workspace role in a Workspace.
_Avoid_: user, participant, collaborator, person

**Right**:
One step of the scale a Member holds on a Board: none, viewer, commentator, editor or full (wire `null`, `viewer`, `commentator`, `editor`, `co_owner`); one scale for `b share`, `b grant` and `b revoke`.
_Avoid_: access (alone), permission, permits (the server's), role (the server's, except Workspace role), grant (for the value)

**Edit right**:
What the Account may do on a Loaded board: read-only or editable; JSON key `edit_right`, code `EditRight`.
_Avoid_: board right, access (alone), readOnly (the server's)

**Workspace role**:
What a Member is in a Workspace: admin, member or guest; JSON key `role`, the server's values.
_Avoid_: right (that is the Board scale), permission

**Sharing**:
The rules a Board is open by, all set with `b share`: Link sharing, the Right of the Workspace's Members, who may copy, delete, use tools or lock content, and the Template link.
_Avoid_: share settings, sharing settings, publicity

**Link sharing**:
The Right that anyone holding a Board's Link has without being a Member: none, viewer, commentator or editor; `b share --link`.
_Avoid_: public access, public link (except the server's `public_access`)

**Thread**:
A discussion on a Board, started by a Thread message on a Thread anchor and resolved or reopened as a whole; the Command group stays `comment`.
_Avoid_: comment (except the Command group `comment`), note (a Diagnostic), conversation

**Thread message**:
One message in a Thread: the first one or a reply.
_Avoid_: comment, message (alone), post

**Thread anchor**:
What a Thread sits on: an Object, or a Point on the Board.
_Avoid_: pin, anchor (alone), attachment

**Thread id**:
The id that names a Thread in `comment reply`, `resolve` and `rm`.
_Avoid_: short id, comment id

**Cover**:
The picture a Board shows in the Workspace tree, set with `b edit --cover`.
_Avoid_: thumbnail (except the server's `thumbnail_updated_at`), preview image

**Board note**:
The text kept with a Board, set with `b edit --note` and read with `b show --with note`.
_Avoid_: description, memo, note (alone)

**Tag**:
A named label with a Color that Objects of a Board carry, set with `o edit --tag` and filtered with `o ls --tag`.
_Avoid_: label, category

**Template link**:
The public link that offers a Board as a Template, switched on and off with `b share --template` (wire `share_as_template`).
_Avoid_: template share, share link

**Backup**:
A copy of a Board document that the server keeps.
_Avoid_: snapshot, dump

**Link**:
The web address of a Board or an Object, in its web or desktop form.
_Avoid_: url (except Base URL and the command name), address (for a Link), href

**Object link**:
The web address an Object opens when clicked, set with `--link URL` on `o add` and `o edit` (`--link ""` removes it); wire `linkTo`, `link_to` in `o ls`. A link inside text is markdown `[text](url)`.
_Avoid_: href, hyperlink, link (alone, for this)

**Base URL**:
The origin of the Holst server that every request and Link starts from.
_Avoid_: host (alone), server url

**Tree change**:
A Change to Tree nodes: create, rename, move, copy, delete, restore, favorite, leave.
_Avoid_: REST change

**Planned change**:
A Tree change worked out as an API request together with its Reply, not yet sent.
_Avoid_: pending change

**Reference**:
What the user types to name one thing: an id, a Link, an id prefix, or part of a name.
_Avoid_: spec, ref, selector

**Alias**:
A short `@N` that stands for a full id in Replies and in argument help; the MCP server makes them, and the Command line refuses an `@N` until it keeps handles.
_Avoid_: handle, short id

**Lookup**:
Finding the one thing a Reference names, trying match rules in order.
_Avoid_: resolve, pick, matching

**Candidate**:
One of several things a Reference could name, shown so the user can tell them apart.
_Avoid_: option, match (alone)

**Choice**:
A Lookup of a Workspace, Board or Folder by name across the Workspaces the Account can read.
_Avoid_: pick, selection

**Search**:
A Name search followed by a Board text search.
_Avoid_: lookup (a Lookup finds one thing)

**Name search**:
Finding Tree nodes whose name matches typed text, best match first.
_Avoid_: hit (alone)

### Board and objects

**Board document**:
The Yjs document that holds a Board's Objects.
_Avoid_: doc, ydoc, state, synced

**State vector**:
Yjs's record of which Board updates a Board document holds.
_Avoid_: version

**Board update**:
The changes of one Yjs transaction to a Board document, sent or received.
_Avoid_: patch, delta, diff, pending, waiting

**Accepted update**:
A Board update the server has taken.
_Avoid_: acked, confirmed

**Loaded board**:
A Board document and the Edit right on it, after a Board load.
_Avoid_: synced board

**Board content**:
A Board's Objects and the Edit right on them.
_Avoid_: content (alone)

**Object**:
One thing on a Board.
_Avoid_: item, element, node, widget, shape

**Object kind**:
What an Object is, named by its kind word: `sticker`, `text`, `shape`, `frame`, `arrow`, `link`, `image`, `file`, `document` (wire `card`), `mindmap` (`mind-map-node`), `task` (`task-card`), `flipcard` (`flip-card`), `kanban`, `table`, `code`, `dice`, `wheel` (`spinner-wheel`), `icon` (`phosphor-icon`), `stamp` (a reaction stamp), `emoji`, `drawing`; the Sticker stack and a kind with no word yet (the wire `task` is one) show as `other` with the wire word. Code and Replies use the kind word, the wire name stays in the wire modules.
_Avoid_: type (alone), object type

**Sticker**:
A square note with text.
_Avoid_: sticky note, note (alone)

**Sticker stack**:
A titled card that holds a pile of Stickers.
_Avoid_: stack (alone)

**Free text**:
Text placed on a Board without a box; kind word `text` (wire `simple-text`).
_Avoid_: simple text, text box, label

**Shape**:
An outlined figure with text: a rectangle, an ellipse, a polygon and the like.
_Avoid_: figure, primitive

**Shape kind**:
The outline a Shape has.
_Avoid_: shape type

**Shape outline**:
A Shape's drawn outline and the Rectangle its text fits in.
_Avoid_: corners

**Frame**:
A titled box that holds the Objects inside it; moving it moves them.
_Avoid_: group, container, section, area

**Frame title**:
The text on a Frame's tab above its top edge.
_Avoid_: label

**Parent frame**:
The deepest Frame that wholly holds an Object.
_Avoid_: container, owner, inside, innermost

**Arrow**:
A line from one Object or Point to another.
_Avoid_: connector, edge, line (alone)

**Arrow end**:
One end of an Arrow: held by an Object on one of its Object sides, or free at a Point.
_Avoid_: tip, endpoint

**Arrow route**:
The path an Arrow is drawn along: its ends and the turns between them.
_Avoid_: route (alone), path

**Arrow line**:
How an Arrow runs: straight, elbow or curved.
_Avoid_: line (alone), arrow type

**Arrowhead**:
The mark at an Arrow end.
_Avoid_: head (alone), tip

**Arrow text**:
An Arrow's Object text, drawn on its line.
_Avoid_: label, arrow label

**Object side**:
One side of an Object's Rectangle, where an Arrow end can be held.
_Avoid_: edge, control

**Image**:
An Object that shows a picture file from the storage.
_Avoid_: picture, photo, asset, media

**File object**:
An Object that holds any other uploaded file.
_Avoid_: attachment

**Object text**:
The text an Object shows, kept apart from its fields.
_Avoid_: content, body, label

**Table track**:
One row or column of a table Object, with its order and its length.
_Avoid_: line

**Layer order**:
The order Objects are drawn in, from the bottom up.
_Avoid_: z order

**Point**:
A place on a Board in board units, both coordinates finite.
_Avoid_: position, coordinates, spot, place

**Size**:
A width and a height in board units, both positive.
_Avoid_: dimensions, extent

**Rectangle**:
An axis-aligned box in board units.
_Avoid_: rect, bounds, box

**Color**:
A palette token, or a stored RGB value, that a fill or a line is painted with.
_Avoid_: colour, ink, fill (alone), stroke (alone)

**Placement**:
Where a new Object goes: its corner and its Parent frame.
_Avoid_: position, slot

**Free corner**:
The top-left corner where a new Object fits inside a Frame without covering others.
_Avoid_: spot, slot

**Author number**:
The number a Board document gives each User who wrote to it.
_Avoid_: user number

**Removal journal**:
The record of removed Objects that a Board's activity panel restores from.
_Avoid_: deletion journal, delete events, history

**Change**:
Something the user asks holst to alter on the server: a Tree change or a Board change.
_Avoid_: mutation, operation, outcome

**Board change**:
One add, edit or remove of a Board's Objects, built on the Loaded board.
_Avoid_: operation, op, apply, fresh, effect

**Changed object**:
An Object a Board change created, changed or removed, with the fields it set.
_Avoid_: touched object, effect

**Remove**:
Taking Objects off a Board, together with a Frame's content and the Arrows they hold.
_Avoid_: delete (for Objects), erase

**Board write**:
Carrying a Board change out: load the Board, build one Board update, send it, and wait until the server accepts it.
_Avoid_: sync, push, apply, commit (Yjs's own word only)

**Release check**:
Whether a built Board change may be sent without a Confirmation.
_Avoid_: send if

**Update receipt**:
What holst knows of one Board update after a Board write: only built, or accepted, with its size and id.
_Avoid_: fate, outcome

**Board status**:
Whether a Board is live or in the Trash when a Board change reaches it.
_Avoid_: placement

**Change report**:
What is told of a Change after it is made.
_Avoid_: outcome

**Risky change**:
A Change that cannot be undone or takes more along than it names, so it needs a Confirmation.
_Avoid_: guarded, dangerous, stake

**Change risk**:
Why a Risky change needs a Confirmation: it cannot be undone, it changes Sharing, it reaches everyone, or it takes a Frame's content.
_Avoid_: action risk, stake, cascade

**Confirmation**:
The user's yes to a Risky change.
_Avoid_: consent, approval, force (except the flag), refusal

**Dry run**:
A Change built and shown in the same words as a real one, and not sent.
_Avoid_: preview, simulate, dry (alone)

**Object query**:
Which Objects to list: of a kind, with a text, one page of the matches.
_Avoid_: filter options, listing options

**Object list**:
The Objects of a Board or an Area with their Links.
_Avoid_: rows, listing (alone)

**Board index**:
The compact index of one Board that Object lists and Board text search read.
_Avoid_: catalog, content index, rows

**Board text search**:
Finding Objects whose Object text matches, across Boards.
_Avoid_: content search, grep, full-text

**Excerpt**:
A piece of an Object text around a match, on one line.
_Avoid_: snippet

### Pictures and files

**Area**:
The part of a Board that is shown or listed: the whole Board, a Rectangle, a Frame, or chosen Objects with Padding.
_Avoid_: selection, region, viewport, part, place

**Scene**:
A Board's Objects indexed for geometry, from which Areas and Pictures are worked out.
_Avoid_: model, layout

**Listed area**:
The Objects an Area covers, with its Rectangle and its Link target.
_Avoid_: area listing

**Link target**:
The Object that a Link to a Picture or an Object list opens.
_Avoid_: anchor

**Padding**:
The margin around an Area, in board units.
_Avoid_: margin

**Picture**:
An Area drawn as SVG or PNG.
_Avoid_: snapshot (except the command name `snap`), screenshot, render, shot

**Picture format**:
SVG or PNG.
_Avoid_: picture (for the format), type (alone)

**Picture scale**:
Pixels per board unit, and the longest side in pixels.
_Avoid_: zoom

**Embed**:
An SVG Picture that carries its Images inside instead of linking to the storage.
_Avoid_: inlined, inline (for an Embed)

**Text layout**:
How text wraps in its Rectangle and which Font size a Sticker takes.
_Avoid_: typesetting, layout (alone), flow

**Font size**:
The size of text, in pixels.
_Avoid_: size (alone)

**Line height**:
The distance between two lines of text.
_Avoid_: line (alone)

**Card style**:
How a drawn card with text looks: its fill, border, corner radius and Font size.
_Avoid_: box style

**Stroke style**:
Whether a line is solid, dashed or dotted.
_Avoid_: line style

**Brief**:
What holst tells about a Link: what it names, its size, what it holds by kind, its Parent frame; code `BoardBrief`, `NodeBrief`.
_Avoid_: info, summary, details, shown

**Save folder**:
The folder one save writes: the Picture as PNG and SVG, the Object list, and the Originals.
_Avoid_: bundle, export (that is a Board file)

**Stored file**:
The file behind an Image or a File object, as the storage keeps it.
_Avoid_: blob, asset

**Original**:
A Stored file's bytes as they were uploaded.
_Avoid_: source file, raw file

**Upload**:
Putting a local file into the storage.
_Avoid_: attach

**Out file**:
A new file that a command writes, never over an existing one.
_Avoid_: destination, sink

**Board file**:
A Board's whole Board document saved to a file, to be written into a Board again.
_Avoid_: dump, saved state, packing, clone

**Compression**:
Whether a Board file or a Backup is gzip or plain.
_Avoid_: packing, packed

### Local data

**Local data**:
Everything holst keeps on this machine.
_Avoid_: storage (except the command name), local files

**Data place**:
One file or folder of Local data, with its role.
_Avoid_: place (alone), stored file

**Partial file**:
A file or folder written beside its target first and renamed over it when complete.
_Avoid_: temp file, scratch

**Lock file**:
The file that the Board cache and the Session store take so that one process at a time writes.
_Avoid_: lockfile

**Clean**:
Removing Local data that the current Login does not need.
_Avoid_: purge, wipe (alone)

**Board cache**:
The Board documents and images holst keeps on disk, per Account and server, within a size limit.
_Avoid_: store, storage, disk cache

**Cached copy**:
One Board's document in the Board cache, with how far it is trusted.
_Avoid_: entry, held, snapshot

**Cached asset**:
An image file the Board cache keeps for drawing Pictures.
_Avoid_: held asset, asset (alone)

**Trust state**:
How far a Cached copy is trusted: unverified, trusted within the Trust window, verified by the server, or read offline.
_Avoid_: freshness, validity

**Trust window**:
How long after its last full download a Cached copy may seed a Board load.
_Avoid_: ttl, expiry, max age

**Seed**:
The Cached copy a Board load starts from, so that only what is missing is downloaded.
_Avoid_: base (alone), warm start

**Eviction**:
Removing Cached copies and Cached assets past the size limit or long unused.
_Avoid_: purge, cleanup (Clean is the user's)

**Board load**:
Reading a Board document: from a Seed and what the server has beyond it, or as a full download.
_Avoid_: fetch, sync, pull, resolve

**Board memory**:
The Boards the MCP server keeps in memory within a byte limit, the least recently read going first.
_Avoid_: cache, LRU, memory (alone)

**Tree memory**:
The Workspace trees and the Workspace list the MCP server keeps in memory for a short time.
_Avoid_: tree cache, cached tree

**Write reach**:
What a sent Change makes stale in the MCP server's memory: one Tree node, one Workspace tree, all of them, or the Workspace list.
_Avoid_: scope (alone)

**Invalidation**:
Forgetting what a Change made stale in the MCP server's memory.
_Avoid_: cache drop

**Generation**:
The counter that orders the MCP server's reads and Invalidations, so that a read older than an Invalidation is dropped.
_Avoid_: epoch, version

**Login epoch**:
The number of the MCP server's installed Login, so that a client from an older one does not save its Rotated session.
_Avoid_: generation

### Machinery

**Surface**:
Where the user meets holst: the Command line or the MCP server.
_Avoid_: frontend, UI, client

**Command line**:
The `holst` program with its Commands and global flags.
_Avoid_: CLI app, frontend

**MCP server**:
The `holst-mcp` program: its Tools on stdio and what it keeps between calls.
_Avoid_: frontend, handler

**Command**:
One entry of the command table: a Command group and a Verb (on the MCP server a Tool and its `action`) with its arguments; `holst <group> <verb>`.
_Avoid_: operation, request, flags

**Command group**:
A set of Commands under one word: `auth`, `workspace` (`ws`), `board` (`b`), `object` (`o`), `template` (`tpl`), `comment`, `backup`, `storage`, `search`, `api`; on the MCP server one Tool.
_Avoid_: category, namespace, module

**Verb**:
The word after the Command group that says what a Command does (`ls`, `show`, `add`, `edit`, `rm`); on the MCP server the `action`.
_Avoid_: subcommand (alone), action (except the MCP argument)

**Command risk**:
What a Command may do: read, write, or destroy; destroy exactly when the Command takes a Confirmation.
_Avoid_: action risk, stake

**Sidebar view**:
The `b ls --view sidebar` listing: Boards and Folders as the web client's sidebar shows them.
_Avoid_: sidebar (alone)

**Signed command**:
A Command that runs with a logged-in Client, checked for risk before the Login is read.
_Avoid_: authenticated command

**Tool**:
One tool of the MCP server.
_Avoid_: function, endpoint, call

**Tool arguments**:
The JSON arguments of one Tool, as its schema states them.
_Avoid_: params, request

**Reply**:
What a Command or a Tool prints for its result.
_Avoid_: response, answer, shown, data

**Change reply**:
How a Tree change is shown: one line naming what changed, a Board's Link under it.
_Avoid_: outcome, shown

**Reply style**:
Compact text with Aliases, or raw JSON.
_Avoid_: form, mode (alone)

**Skipped**:
A Workspace or Board a Search could not read, named beside the hits (`skipped_workspaces`, `skipped_boards`, `SkippedBoard`, `SkipReason`).
_Avoid_: unread (for a Search), ignored, failed

**Items file**:
The JSON array or JSON lines file (`-` for stdin) that `--items` reads: one add or edit per entry, keys the flags of one add or edit; the MCP server takes at most 100 per call, the Command line writes batches of 100.
_Avoid_: batch file, bulk file

**Event log**:
What a Workspace's Members did, newest first: `ws log` and `ws show --with activity`.
_Avoid_: activity log, audit log, history

**API token**:
A token the Account made for the Holst API, listed by `auth status --with tokens`; it is not the Session.
_Avoid_: token (alone), key

**Server session list**:
The Server sessions the Account holds, read with `auth status --with sessions` (read only; none is ended).
_Avoid_: session list (alone), devices

**Output format**:
How a Command prints: plain text, JSON or pretty JSON.
_Avoid_: mode (alone), style (alone)

**Diagnostic**:
A note or warning line on stderr.
_Avoid_: message (alone), log line, label (alone)

**Hint**:
One thing the user can do, shown under an error in the Surface's words.
_Avoid_: tip, suggestion, help, words (alone)

**Surface wording**:
The words one Surface uses for Confirmations, Hints and errors.
_Avoid_: hint wording

**Input**:
A value given by the user or the environment that cannot be used as given; nothing is sent.
_Avoid_: bad value, invalid input, validation

**Form**:
The Questions a Login asks on the terminal.
_Avoid_: prompt, wizard, dialog, hush

**Question**:
One thing a Form asks for: an Email address, a Login code or a Password.
_Avoid_: prompt, field

**Answer**:
What the user types for a Question.
_Avoid_: row, reply (alone), stroke (alone)

**Invalid answer**:
Why a typed Answer cannot be used.
_Avoid_: problem, rejected, refused

**Spinner**:
The turning glyph a Form shows while it waits.
_Avoid_: frame

**Coloring**:
Whether printed text is plain or colored.
_Avoid_: colour, ink

**Terminal type**:
Whether the terminal named by the environment can draw escapes, or is dumb.
_Avoid_: term (alone)

**Run**:
One start of `holst`, from its arguments to its exit code.
_Avoid_: env, environment, invocation, session, moment

**Run phase**:
Whether a Run waits for an Answer, so that a Signal ends it at once, or works and stops itself.
_Avoid_: moment

**Settings**:
What flags and the environment configure for a Run.
_Avoid_: config, options, env

**Signal**:
An operating system signal that stops a Run, and the exit it causes.
_Avoid_: event, notification

**API request**:
A call to the Holst server described as data: method, path, query, body, and how much of the Session it carries.
_Avoid_: call, req, auth

**Query parameter**:
One name and value of an API request's query.
_Avoid_: entry, key

**Authorization**:
How much of the Session an API request carries: all of it, the cookies only, or nothing.
_Avoid_: auth, bearer

**Endpoint**:
One REST operation of the Holst server, as holst declares it.
_Avoid_: route (alone), api (alone)

**Response**:
What came back from the server for a request.
_Avoid_: reply, answer

**Exchange**:
One request end to end: its redirects, its retries and the final Response.
_Avoid_: leg (alone), round trip

**Transport**:
What sends one HTTP exchange.
_Avoid_: network

**Holst server**:
The server holst talks to: its REST API, its update links and the Board socket's address.
_Avoid_: board server

**Client**:
What sends API requests for one Run: the Base URL, the Transport and the Session state.
_Avoid_: conn, http client

**Handed client**:
A Client the MCP server hands to one Tool call, with the Generation and Login epoch it was taken at.
_Avoid_: conn, connection

**Board socket**:
The WebSocket that loads a Board document and carries Board updates.
_Avoid_: ws, websocket, hud

**Socket session**:
The Board socket's address with the Session it sends.
_Avoid_: socket login

**Remote board**:
A Board together with what is needed to reach it over the Board socket.
_Avoid_: board link

**Clock**:
Where the current time comes from: the system's, or a test's.
_Avoid_: now, time source

**Id source**:
Where fresh ids for new Objects and Board updates come from.
_Avoid_: rng, id generator

**Scenario**:
One test story run on both Surfaces against one Fake server.
_Avoid_: flow, story (alone)

**Observation**:
What a Surface showed in a Scenario, in words both Surfaces share.
_Avoid_: shown, seen

**Fake server**:
The stateful stand-in for the Holst server that Scenarios and end-to-end tests run against.
_Avoid_: mock, stub

**Fixture**:
What the Fake server holds when it starts.
_Avoid_: seed

## Server words

The Holst server's own words and the term each one is in holst-rs. They appear only where holst-rs reads or writes the server's paths, JSON keys and Board socket frames, and in the raw API request command and Tool.

| server word | term |
|---|---|
| `channel`, `channel_id`, `NewChannel`, `parent_type`, `order_index` | Board, Folder, Tree node, Parent, Sidebar order |
| `workspace`, `workspace_id`, `tree` | Workspace, Workspace tree |
| `current_user`, `user` | Account, User |
| `get_public_options`, `hasAccount`, `viaPassword`, `viaTemporaryPassword` | Login methods |
| `send_temporary_password`, `with_email`, `app_auth` | Login code, Login |
| `Bearer`, `token`, `sessionToken`, `social-auth-store`, `Set-Cookie` | Session |
| `logout`, `logout_single`, `session/list` | Logout, Server session |
| `permits`, `channel_user`, `co_owner` | Right, Member |
| `role` | Workspace role (Workspace), Right (Board) |
| `readOnly` | Edit right |
| `public_access`, `public` | Link sharing, Sharing |
| `linkTo` | Object link |
| `share_as_template` | Template link |
| `thread`, `thread_list`, `channelId` | Thread, Thread id |
| `backup`, `snapshot`, `snapshotId` | Backup |
| `org`, `organization` | Organization |
| `board_template`, `slug` | Template, Template category |
| `hud`, `hud/ws`, `RequestDocument`, `Response`, `Ack`, `leftUpdates`, `updateId`, `encV2`, `serverTs`, frame | Board socket, Board update, Accepted update |
| `Update` with `Inline` or `Link`, `hud/upload`, `hud/download`, `cdns-edge` | Board update |
| `Y.Doc`, `objects`, `documents`, `documentId`, `wrapper`, `paragraph` | Board document, Object, Object text |
| `usersIds`, `created`, `updated` | Author number |
| `delete-events-metaStore`, `cnt`, `s`, `unlock` | Removal journal |
| `type`, `sticker`, `sticker-stack`, `simple-text`, `shape`, `frame`, `arrow`, `image`, `file` | Object kind and its kinds |
| `position`, `width`, `height`, `zIndex`, `parentId`, `labelText` | Point, Size, Layer order, Parent frame, Frame title |
| `fillColor`, `strokeColor`, `strokeWidth`, `strokeStyle`, `shapeType`, `arrowType`, `start`, `end`, `objectId`, `control` | Color, Stroke style, Shape kind, Arrow line, Arrow end, Object side |
| `fileName`, `displayFileName`, `originalName`, `imageType`, `storage-edge`, `upload_file` | Stored file, Original, Upload |
| `errCode`, `msg`, `vars`, `unauthorized`, `forbidden`, `internal-error` | an error's own words, kept as its detail |
| `gapi`, `api` | Endpoint |
| `Subscription` | none: a billing value shown as the server gives it |

## Excepted words

Names that no term covers, each standing in its ordinary meaning.

| words | meaning |
|---|---|
| `Error`, `Result`, `Err`, `Output`, `Value`, `Target`, `BoxFuture`, `Infallible`, `Mutex` | std's and each crate's boundary names |
| `Cache`, `Backend`, `Console`, `Flag`, `Csi`, `Uuids`, `CivilDate`, `FileAccess`, `Sealed`, `StartError`, `Paint`, `Shown` | the Board cache's handle, the MCP server's store-and-client factory, the process's terminal, a two-state flag, a terminal control sequence, packed UUIDs, a calendar day, a file mode, the sealed-trait pattern, why the MCP server did not start, SVG's paint, an error already printed |
| `Finite`, `Positive`, `CheckAll` | value rules: a checked number, independent checks reported together |
| `Cli`, `*Args`, `*Arg`, `*Command` | clap's words: the parser, one Command's or Tool's arguments, one flattened argument, a group of subcommands |
| `Refresh`, `Offline`, `Caching`, `CleanMode`, `SaveLogin`, `RawWrite`, `PipeEscape`, `TrashWalk` | flag enums, named after what they turn on |
| `Unverified`, `Trusted`, `Verified`, `Offline`, `LoginStart`, `ReadyForSecret`, the Login steps, the socket write's markers | typestates, named by their step |
| `row` | a table's row only |
| `millis` | std's word for milliseconds |
| test and bench helpers: `Probe`, `Fake`, `FakeServer`, `Script`, `Spy`, `Screen`, `TestDirectory`, `TempKeychain`, `TempCache`, `SetupError`, `Measurement`, `CannedResponse` and the like | stand-ins named by what they stand in for |
