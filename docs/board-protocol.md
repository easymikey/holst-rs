# Board protocol

Board objects (stickers, texts, shapes, frames, arrows) do not live in REST. They live in a Yjs document that the web client holds over the WebSocket `wss://app.holst.so/hud/ws`. The protocol is not public and was reconstructed from the web client's bundle and live sessions; it may change without notice. Code: `holst-core/src/protocol/` (frames, `lib0`, field names), `domain/board/` (model, reading), `domain/change/` (building changes, pure), `remote/socket/` (socket, upload, waiting for the ack).

## Transport

- Address `wss://<host>/hud/ws`. The handshake carries the session cookies and `User-Agent`. Authorisation is in the first frame: JSON `{"token":"<bearer>"}`. The token is a secret and never reaches logs or error texts.
- One WebSocket message (binary) may hold several frames. A document is addressed by name; a board's name is the board UUID. Only updates with the board name go into the board document. Frames of other documents are skipped with a debug log. From the zero name only `Response` and `Ack` are accepted.
- Awareness (cursors, presence) is needed neither for reading nor for writing.
- Encoding is lib0: `varUint` is LEB128, `varInt` has a sign bit, strings and byte arrays carry a `varUint` length.

| Frame type | Direction | Body |
|---|---|---|
| 0 Update | server to client | server time, `leftUpdates`, `updateId`, kind (0 inline bytes, 1 link to a snapshot) |
| 0 Update | client to server | `updateId`, kind (0 inline bytes, 1 link: size and `<board>/<updateId>`) |
| 1 RequestDocument | client to server | JSON with the token |
| 2 Ack | server to client | `updateId` of our update |
| 3 Response | server to client | `{"readOnly":false,"status":"ok"}` or `{"error":"forbidden"}` |

Loading: the client sends `RequestDocument`; the server answers with `Response` and a series of updates whose `leftUpdates` counts down. Only a board update with `leftUpdates` 0 finishes loading, and only when the document already holds everything the state vectors of all board frames announced. Then the document is checked for holes; if there are any, the result is `BoardIncomplete` and nothing is read or written. A link update points to a snapshot on `cdns-edge.app.holst.so`; it is fetched without cookies and token, and only from the same host or its subdomains.

## The Y.Doc model

| Root | Type | Contents |
|---|---|---|
| `objects` | Y.Map | object id to a Y.Map of fields (nested values such as `position` are plain JSON) |
| `documents` | Y.Map | document id to a Y.XmlText in the slate-yjs layout |
| `usersIds` | Y.Map | user UUID to author number |
| `task-card-statusesStore`, `task-card-tagsStore` | Y.Map | task statuses, tag definitions |
| `delete-events-metaStore` | Y.Map | deletion journal |

A new board has no roots; they appear with the first write. Coordinates are absolute: for an object inside a frame `position` is in board coordinates and `parentId` is the frame. The text of an object is not in its fields: `documentId` points into `documents`. Text is a root XmlText with one embedded XmlText per block (`type`, `key` attributes); marks are boolean attributes on a leaf. Block types: `paragraph`, `heading-one` to `heading-three`, `ul-list-item`, `ol-list-item`, `checkbox-list-item`, `divider`, `code-block`, `link`. Sticker and text use one `wrapper` for all paragraphs; cards, notes and cells use one `wrapper` per block.

### Sticker (reference)

| Field | Example | Meaning |
|---|---|---|
| `id`, `type` | UUID, `sticker` | key in `objects`, kind |
| `documentId` | UUID | text in `documents` |
| `position` | `{x:100, y:100}` | top-left corner |
| `width`, `height` | `192`, `192` | size |
| `zIndex` | fractional | layer order, larger is higher |
| `fillColor` | `{color:"yellow4", opacity:1}` | palette token |
| `created`, `updated` | `{a, t}`, `null` | author number and time in ms; last change |
| `textScale`, `showAuthor`, `parentId` | number, bool, UUID | optional |

### Other types

A key may be absent while the value is the default; the reader accepts absence and an empty string. Common to all kinds: `id`, `type`, `position`, `zIndex`, `created`, `updated`, `parentId` (a frame; for a cell the table; for a locked object its `group`), `linkTo`. `holst object add` writes `sticker`, `simple-text`, `shape`, `frame`, `arrow`, `link`, `image` and `file`; other kinds are only read, and unknown names stay as `ObjectKind::Other`.

| Kind | Fields that matter |
|---|---|
| `sticker` | `fillColor`, `documentId`, `textScale`, `fontSize`, `horizontalAlign` (`left`, `right`, `justify`; absent is centred), `verticalAlign` (`top`, `bottom`; absent is middle), `tagIds` |
| `simple-text` | `fontFamily`, `lineHeight`, `documentId`; no size |
| `shape` | `shapeType` (about 60 names: camelCase basics, then `flowchart-*` and `bpmn-*`; unknown draws as `square`), `fillColor`, `strokeColor`, `strokeWidth`, `documentId` |
| `frame` | `width`, `height`, `fillColor`, `labelText`; lowest `zIndex`; frames nest |
| `arrow` | `arrowType` (`straight`, `elbow` default, `curved`), `strokeStyle` (`dashed`, `dotted`; absent is solid), `strokeWidth`, `strokeColor`, `arrowheadStart`, `arrowheadEnd`, `start`, `end`; `position` is always 0,0 |
| `card` | `width`, `fixedSize`, `colorToken`, `documentId` |
| `image` | `name` (from `POST /gapi/upload_file`), `originalName`, `imageType`, `width`, `height`, `naturalWidth`, `naturalHeight`, `cropTransform` |
| `file` | `fileName`, `displayFileName`, `fileType` (MIME), `fileSize`, `displayType`; no size |
| `link` | `link`, `displayType` (1 card, 2 iframe), `linkInfo` |
| `table`, `table-cell` | `row-<n>` and `column-<n>` keys with `index` and `height` or `width`; cells carry `rowId`, `columnId`, `documentId`, `parentId` = the table |
| `code` | `width`, `theme`, `lang`; the text is the root Y.Text `code-mirror-<documentId>` |
| `drawing` | `algorithm` (`lazy`), `path` (flat relative `x,y` pairs), `strokeWidth`, `color` (integer RGB), `alpha` |
| `stamp` | `data` `{type: "imageStamp", url}` or `{type: "textStamp", text}` |
| `reaction-stamp` | `stampKey` (`like`, `heart`, `star`, `+1`, `dislike`, `check`, `cross`), `rotation` |
| `mind-map-node`, `task-card`, `kanban`, `flip-card`, `dice`, `spinner-wheel`, `phosphor-icon`, `group` | read only; fields are in the web client's schemas |

Arrow details: arrowheads are `none`, `figma`, `miro`, `arc`, `triangle`, `triangle-filled`, `circle`, `circle-filled`, `rhombus`, `rhombus-filled`, `stick`, `stick-stick`, `triadius`, `triadius-stick`, `triadius-circle`, `stick-circle`, `slash`; absent means none at the start and `miro` at the end. An end is `{point}` or, attached, `{objectId, point, relativePoint, control}` where `control` is the side (1 top, 2 right, 3 bottom, 4 left). An attached end is read from the object, not from `point`.

Colours are tokens `<name><shade>` (`yellow4`, `gray12`); `color: null` is transparent. `drawing.color` and `task-card.color` are integer RGB. Tags: definitions are `task-card-tagsStore[<id>] = {id, title, color}`; an object carries `tagIds`; unknown ids are treated as absent. The board note is the root XmlText `board-note`. Comments live in a separate room `<board>$comments` on the same socket (`commentsStore`, one nested Y.Map per thread or reply).

### Author number (`usersIds`)

`created.a` and `updated.a` are not UUIDs but an index into `usersIds`: a 31-bit hash of the user's UUID (a cyrb53 variant with `0xdeadbeef ^ salt` and `0x41c6ce57 ^ salt`, `Math.imul` over UTF-16, result `& 0x7fffffff`), where `salt` grows from 0 while another user holds the number. If we are not in `usersIds`, an entry is added in the same transaction as the write.

## Writing

- The update body is Yjs v2 and holds exactly the changes of one transaction (`TransactionMut::encode_update_v2`), not a diff against the state vector. `board import` is the one exception.
- `updateId` is a random integer below 2^52. Success is reported only after an `Ack` with the same id. If the update was sent or an upload may have completed and no ack came, the result is `HudUnconfirmed`: re-read the board before retrying, or a retry duplicates objects.
- An update over 4096 bytes goes by `POST /gapi/hud/upload?documentName=<board>&updateId=<id>` (Bearer, `application/octet-stream`) followed by a link frame.
- The client id of our `Doc` is 31-bit and chosen outside the state vector of the loaded board.
- A change is built on the board as loaded; edits made by others in between are merged by the CRDT but not seen by the change.
- Editing text clears the existing XmlText and writes it again, one wrapper per line.
- A new object goes into the deepest frame that entirely contains it. `object add --frame` requires the object to lie inside that frame, otherwise `OutsideFrame`. Moving a frame moves its subtree and the arrows with both ends in it. Leaving all frames writes `parentId: null`.
- Deletion is hard: the key is removed from `objects`, texts in `documents` stay. A frame deletes its subtree, and arrows with a deleted end are deleted in the same update. Every deletion also writes one journal entry `{id, a, t, cnt, s, type?, unlock?}` into `delete-events-metaStore` in the same transaction (`s` holds the old fields, `a` the author number, `t` the time in ms, `id` a 5-character hash of the sorted deleted ids). The entry moves the state vector so caches refetch, and lets the web client restore the objects.

## Formats and inputs refused

- A board frame of an unknown type is `MalformedFrame`; its length is unknown, so the rest of the message is not read.
- A link update to a foreign host is `ForeignLink`. `readOnly: true` stops the write (`BoardReadOnly`).
- An object whose coordinates or sizes are not finite or exceed 10^12 is not drawn and not selected. An arrow whose end refers to a deleted object, a group, an arrow or a cell without a table is not drawn.
- Snapshot options out of range (`--max-side` 16 to 16384, `--scale` above 0 and at most 100, `--padding` 0 to 100000) are a `SnapshotOption` error before the board loads. Pictures are limited to 4096 by 4096 pixels in total.
- Characters forbidden in XML are dropped from SVG text. SVG is never printed to a terminal. Control and bidi characters in board text are replaced when printed as text.
- Gzip input to `board import` is unpacked up to 1 GiB with a CRC check.

## Links

An object in the browser is `https://app.holst.so/board/{board}?objectId={object}`; `/share/b/{board}` redirects to it. Desktop is `/open-in-desktop?boardId={board}&objectId={object}`. The board reads no other parameters, so a link to an area points to the smallest frame that contains it, otherwise to the nearest object.
