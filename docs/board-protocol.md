# Board contents: the hud protocol and the object model

Board objects (stickers, texts, shapes, frames, arrows) do not live in REST. They live in a Yjs document that the web client holds over the WebSocket `wss://app.holst.so/hud/ws`. This document is the result of analysing the bundle `main.5b1ea1b9.js` and live sessions: reading (stages 1-2) and writing (stage 3). Code: `holst-core/src/protocol/` (`hud.rs` - frames, `lib0.rs`, `document.rs` - Yjs field names), `domain/board/` (model, reading), `domain/change/` (building changes, pure), `remote/socket/` (socket, upload, waiting for ack).

## Transport

- Address: `wss://<host>/hud/ws` (`ws://` for `http://`). The handshake carries the session cookies and `User-Agent`, like a browser.
- Authorisation is in the subscribe frame: JSON `{"token":"<bearer>"}`. The token is a secret: it is kept in `SecretString`, `HudLogin` has no `Debug`, and the token never reaches logs or error texts.
- One WS message (binary) may contain several frames in a row.
- A document is addressed by name: the name of a board document is the board UUID. The web client opens a separate socket per document (the board, `<board>$state`, `<board>$comments`) and accepts only frames with its own name and with the zero name `00000000-0000-0000-0000-000000000000` (it treats those as its own); on a frame with a foreign name it writes `console.warn` and drops the rest of the WS message.
- We are stricter: only Updates with the board name go into the board document. An Update with the zero name and any frames of other documents are not merged into the board and do not finish loading; they are skipped with a log entry (`debug`). From the zero name only `Response` (a refusal) and `Ack` are accepted.
- Awareness (cursors, presence) is needed neither for reading nor for writing: verified live, the server accepts an Update and answers with Ack without a single awareness frame. The bundle's hud provider has no awareness at all.

## Encoding (lib0)

| Type | Format |
|---|---|
| varUint | LEB128: 7 bits each, the high bit means "more bytes follow" |
| varInt | first byte: 6 bits of value, bit 6 is the sign, bit 7 is continuation; then as varUint |
| varString / varUint8Array | varUint length + bytes (a string is UTF-8) |

## Frames

Server frame: a `varString` - if it is `encV2`, a `varString` with the document name follows (v2 encoding), otherwise the string itself is the name (v1). Then a `varUint` type:

| Type | Direction | Body |
|---|---|---|
| 0 Update | server → client | serverTs varUint, leftUpdates varInt, updateId varUint, [v2: stateVector bytes], kind varUint (0 Inline: bytes / 1 Link: string) |
| 0 Update | client → server | updateId varUint, kind varUint: 0 Inline - bytes; 1 Link - size varUint, then string `<document>/<updateId>` |
| 1 RequestDocument | client → server | JSON `{"token":…}` (varString), then `0` |
| 2 Ack | server → client | updateId varUint: confirmation of our update; live, the id in Ack always matched the one sent |
| 3 Response | server → client | JSON: `{"readOnly":false,"status":"ok"}` or `{"error":"forbidden"}` |

Loading: the client sends `RequestDocument`, the server answers with `Response` and a series of `Update`s; `leftUpdates` decreases, `0` is the last update of the initial load. When reading, we close the socket after it; when writing, we keep it open until Ack.

How the web client decides that loading is over (`handleSyncUpdate` in `main.5b1ea1b9.js`): the first Update with `leftUpdates === 0` that arrives in the `Syncing` state, and nothing else is checked (neither the document name beyond the filter above, nor `pendingStructs`). It accumulates the stateVector of frames (or `to` from the update metadata) in `serverStateVectorM` only to send the server its own diff; for live updates after loading it does not look at `leftUpdates` at all. So a live update with someone else's edit and `leftUpdates` 0 in the middle of loading would finish loading for it too.

Our rule (`remote/socket`) is stricter:

- Only a board Update with `leftUpdates` 0 finishes loading, and only when the document already contains everything the stateVectors of all board frames announced (their maximum per client); if not, we keep reading. So a live update in the middle of loading (its stateVector is the server state, larger than ours) does not cut it short;
- after that the document is checked for holes: updates waiting for missing ones (`has_missing_updates`), and blocks that went beyond the state vector (yrs inserts an update without dependencies when earlier ones are missing, leaving a hole - visible in `insertions`); if there are holes - `BoardIncomplete`, nothing is read or written, `board export` creates no file.

Live: a large shared board - one Link update with `leftUpdates` 0 and a stateVector of 304 clients, after applying it the document covers it, 4402 objects; on a new board after four writes the server returns one merged update with the stateVector of all four clients.

An unknown or inaccessible board: `{"error":"internal-error"}` (for someone else's board - `forbidden`).

Link update: `Payload::Link` is the address of a snapshot on `cdns-edge.app.holst.so`. It is downloaded with GET without cookies and token; the client accepts only the same host or its subdomains with the same scheme (`BaseUrl::trusted_update_link`), otherwise `ForeignLink`.

## The Y.Doc model

| Root | Type | Contents |
|---|---|---|
| `objects` | Y.Map | object id → Y.Map of the object's fields (nested values such as `position` are plain JSON values, not Y types) |
| `documents` | Y.Map | documentId → Y.XmlText (Slate-yjs): root → `wrapper` → `paragraph` → text, formatting as attributes |
| `usersIds` | Y.Map | user UUID → author number (`created.a`, `updated.a`) |
| `task-card-statusesStore` | Y.Map | task card statuses |
| `delete-events-metaStore` | Y.Map | deletion journal for the «Активность» (Activity) panel and for restoring (see "Deletion journal"); we write it too |

A new board has an empty document: there are no roots at all, they appear with the first write.

The text of an object is not in its fields: the field `documentId` points to an entry in `documents`. We join paragraphs with `\n`. The structure of the text as written by the web client and the official MCP: a root XmlText without attributes, in it one embedded XmlText per block with XML attributes `type=wrapper`, `key=<uuid>`; the block (an embed with `type=paragraph`, `key=<uuid>` and text inside) comes first in the wrapper. Nesting is a wrapper inside the parent's wrapper after its block, except for stickers and task titles, which have no nesting (`main@4868180`, `main@4996277`). The lines of our text are written as separate wrappers in a row. Embeds have no formatting attributes.

Coordinates are absolute: for objects nested in a frame `position` is in board coordinates, and `parentId` is the id of the frame.

### Sticker (reference)

The reference is a sticker `<sticker-id>` on a board `<board-id>`, created by the official MCP: corner (100, 100), 192×192, `yellow4`, text "zz-test: sticker from Claude Code". `holst object ls` shows it on one line with `@N`, kind, corner, size, colour, text and link.

| Field | Example | Meaning |
|---|---|---|
| `id` | `<sticker-id>` | object UUID, also the key in `objects` |
| `type` | `sticker` | object kind |
| `documentId` | UUID | text in `documents` |
| `position` | `{x:100, y:100}` | top-left corner, board coordinates |
| `width`, `height` | `192`, `192` | size |
| `zIndex` | fractional | layer order, larger is higher |
| `fillColor` | `{color:"yellow4", opacity:1}` | palette token |
| `created` | `{a:<number>, t:<ms>}` | author (index into `usersIds`) and time |
| `updated` | `null` | last change |
| `textScale` | number | text scale |
| `showAuthor` | bool | author caption |
| `parentId` | UUID or absent | parent frame |

The fields `showAuthor` and `parentId` of a sticker are optional: stickers created by the official MCP do not have them.

### Other types

Object kinds and their fields follow the documents written by the web client and the official MCP (checked against document dumps after `createText/createShape/createFrame/createArrow` and manual passes 1-10 through the live interface; values from the bundle carry a reference `main@N` to an offset in `main.a33651a4.js`). Below is one table per kind; names of fields and values are as in the document. Reading rule: a key may be absent while the value is the default; the reader accepts both absence and an empty string.

Fields common to all kinds:

| Field | Value | Meaning |
|---|---|---|
| `id` | UUID | key in `objects` |
| `type` | string | kind (the heading of the table below) |
| `position` | `{x, y}` | top-left corner, board coordinates; for objects nested in a frame also board coordinates, not the frame's |
| `zIndex` | fractional | layer order |
| `created` | `{a, t}` | author (number from `usersIds`) and time, ms |
| `updated` | `null` or `{a, t}` | last edit; not every edit moves it (text marks, `fontSize`, `linkTo` do not; alignment, colour do) |
| `parentId` | UUID, `null` or absent | parent frame; for a cell - the table, for a column card - the kanban, for a mind map child - the parent, for a locked object - the `group` |
| `linkTo` | string | link to an object (a badge in the corner); on a sticker, `card`, `flip-card`; after the link is removed `""` stays |

`holst object add` writes `sticker`, `simple-text`, `shape`, `frame`, `arrow`, `link`, `image` and `file`; other kinds are only read.

#### `sticker`

| Field | Value | Meaning |
|---|---|---|
| `width`, `height` | 192×192 (can be 1:1 or 2:1) | size |
| `fillColor` | `{color, opacity}` (`yellow4`) | palette token |
| `documentId` | UUID | text in `documents` |
| `textScale` | number (1) | text scale |
| `fontSize` | number | font size; absent - «Авто» (Auto, by size) |
| `horizontalAlign` | `left`/`right`/`justify` | absent - centred |
| `verticalAlign` | `top`/`bottom` | absent - middle |
| `showAuthor` | bool | author caption |
| `tagIds` | array of UUIDs | tags (below); the key appears with the first tag |

#### `simple-text`

| Field | Value | Meaning |
|---|---|---|
| `fontFamily` | `"Inter"` | font |
| `lineHeight` | `"150%"` | line spacing |
| `documentId` | UUID | text; no width or height, the size follows the text |

#### `shape`

| Field | Value | Meaning |
|---|---|---|
| `shapeType` | see "Shapes" | shape kind |
| `width`, `height` | fractional (144.87×143.68) | size; defaults per kind in the table below |
| `fixedSize` | `true` | |
| `fillColor` | `{color: "white6", opacity: 1}` | fill |
| `strokeColor` | `{color: "gray12", opacity: 1}` | line |
| `strokeWidth` | `2` | |
| `documentId` | UUID | label |

`shapeType` has 62 names (`main@4747020`); in the «Все фигуры» (All shapes) panel two lanes (horizontal and vertical) are added to them, which insert not a shape but a `table` of two columns and two rows (`main@3746041`). For an unknown name the client draws `square`. Basic shapes are written in camelCase without a prefix, the others as `<group>-<name>`; the default label is the text in `documentId`.

| Group | `shapeType` (default size) |
|---|---|
| Basic | `square` 144×144, `roundedRectangle` 144×144, `ellipse` 144×144, `triangle` 168×144, `diamond` 144×144, `invertedTriangle` 168×144, `leftParallelogram`, `rightParallelogram`, `basic-speech-bubble`, `basic-arrow-right`, `basic-arrow-left`, `basic-cloud` (176×144), `basic-arrow-left-right` 200×144, `basic-star` 156×144, `basic-pentagon` 150×144, `basic-hexagon` 168×144, `basic-octagon` 144×144, `basic-trapezoid` 176×144, `basic-cross` 144×144 |
| Flowchart | `flowchart-process`, `-decision` (200×128), `-terminator` (176×106), `-predefined-process`, `-document`, `-multiple-documents`, `-manual-input`, `-preparation`, `-data`, `-database` (144×128), `-direct-access-storage`, `-internal-storage`, `-manual-operation`, `-delay`, `-stored-data`, `-merge`, `-connector` (148×148), `-or` and `-summing-junction` (86×86, no text), `-display`, `-off-page-connector`, `-left-curly-brace-annotation`, `-right-curly-brace-annotation`, `-comment`, `-note`; the rest 176×128 (some 168-188), the label is "Process", "Decision" … |
| BPMN | `bpmn-task`, `-transaction`, `-event-subprocess`, `-call-activity` (168×128), `-event-start`, `-event-start-non-interrupting`, `-event-intermediate`, `-event-intermediate-non-interrupting`, `-event-end` (40×40), `-conversation`, `-call-conversation` (46×40), `-gateway` (48×48), `-data-object` (60×80), `-data-store` (144×118), `-empty-pool` (200×40), `-group` (200×384), `-annotation` (144×118), `-message` (44×33) |

#### `frame`

| Field | Value | Meaning |
|---|---|---|
| `width`, `height` | | size |
| `fillColor` | `white3` | fill |
| `labelText` | string | title |

`zIndex` is below all others (`generateFirstZIndex`). Frames nest inside each other.

#### `arrow`

| Field | Value | Meaning |
|---|---|---|
| `position` | `{0, 0}` always | the geometry is in `start` and `end` |
| `arrowType` | `straight`, `elbow` (default), `curved` | line kind |
| `strokeStyle` | `dashed`, `dotted`; absent - solid | |
| `strokeWidth` | `1`, `1.5`, … `17` (nine steps); absent - the third | |
| `strokeColor` | `{color: "gray10"}`; after a colour is chosen `{color: "red9", opacity: 1}` | |
| `arrowheadStart`, `arrowheadEnd` | `none`, `figma`, `miro`, `arc`, `triangle-filled`, `triangle`, `circle-filled`, `circle`, `rhombus-filled`, `rhombus`, `stick`, `stick-stick`, `triadius`, `triadius-stick`, `triadius-circle`, `stick-circle`, `slash` | arrowheads; absent - none at the start, `miro` at the end (`main@4043026`) |
| `start`, `end` | `{point: {x, y}}`; attached: `{objectId, point, relativePoint, control}` | `relativePoint` - fractions of the object's frame (`{x: 1, y: 0.5}` - the middle of the right side), `control` - the side (1 top, 2 right, 3 bottom, 4 left) or `null` |
| `parentId` | | the common frame of the ends |

An end dropped on the body of an object is `{objectId, point, control: null}` without `relativePoint`; the client routes the line to the nearest side. A new arrow takes the style of the previous one. An end attached to an object is read from the object, not from `point`: when an object is moved the client changes only the object, and `point` in the arrow stays as it was.

#### `card` (document)

| Field | Value | Meaning |
|---|---|---|
| `width`, `height` | 320, grows with the text | size |
| `fixedSize` | bool | height follows the text or is fixed |
| `colorToken` | `white1` by default, `blue3`… | colour (not `fillColor`) |
| `documentId` | UUID | text (blocks - see "Texts (rich text)") |

#### `image`

| Field | Value | Meaning |
|---|---|---|
| `name` | `<uuid>.png` | response of `POST /gapi/upload_file`; the address is `STATIC + name` |
| `originalName` | `zz.png` | original name |
| `imageType` | `image/png` | MIME |
| `width`, `height`, `naturalWidth`, `naturalHeight` | pixels | |
| `isLoading` | `false` | |
| `cropTransform` | | crop |

#### `file`

One kind for PDF, documents, presentations, video, audio and any file.

| Field | Value | Meaning |
|---|---|---|
| `fileName` | `<uuid>.pdf` | name in storage |
| `displayFileName` | `zz.pdf` | original name |
| `fileType` | MIME | `application/pdf`, `video/mp4`, … - not a menu category |
| `fileSize` | bytes | |
| `displayType` | `content` (pdf, docx, pptx, mp4, mp3), `stamp` (the rest) | preview or icon card |
| `pagesInfo` | `{count, valid}` | pdf, docx, pptx |
| `pageSize` | `{width, height}` | preview page size |
| `videoInfo` | `{previewName, originalWidth, originalHeight, width, height, duration}` | video; `previewName` is a second `upload_file` (a frame drawn by the client) |
| `audioInfo` | `{duration}` | audio |

There are no sizes (`width`, `height`). Upload is `POST /gapi/upload_file?channel_id=<board>` (multipart), the response is `{name: "<uuid>.<extension>"}`.

#### `link`

| Field | Value | Meaning |
|---|---|---|
| `link` | URL | the page or the `src` of a pasted iframe |
| `displayType` | `1` card, `2` live iframe | |
| `linkInfo` | `{link, title, description, faviconKey, imageKey, embedInfo}` | a copy of the response of `GET /gapi/link_preview?link=…`; on refusal (400) the fields are `null`, the card shows the URL |
| `linkInfo.embedInfo` | `{type, version, title, providerName, thumbnailKey, thumbnailSize, html, width, height}` | `html` is the embed code as is |

Pasting a link on an empty canvas makes a `link` card (`displayType: 1`) without `documentId` and sizes; pasting into editable text makes an inline link (below).

#### `table` and `table-cell`

| Field | Value | Meaning |
|---|---|---|
| `strokeColor`, `fillColor` | `{color, opacity}` (`gray7`, `white6`) | lines and fill |
| `fillColorIndex` | `1` | |
| `row-<n>` | `{height, minHeight, index}` | one key per row, on the table itself; `index` is a fractional order key (`2^40`·n) |
| `column-<n>` | `{width, index}` | one key per column; a new number need not be the next one |
| `table-cell` | an object with `parentId` = the table, `rowId`, `columnId` (numbers `<n>`), `documentId`, `position {0, 0}`, `zIndex 0` | created at the first edit of the cell |

A table has no size (the sum of columns and rows). A lane column: `textRotation: -90`.

#### `code`

| Field | Value | Meaning |
|---|---|---|
| `width` | 500 | height follows the number of lines |
| `scale` | 1 | |
| `theme` | `light` (default), `dark` | |
| `lang` | `typescript`, `javascript`, `python`, `cpp`, `java`, `go`, `rust`, `plain_text`, `swift`, `ruby`, `sql`, `html`, `css`, `json`, `kotlin`, `bash`, `php`; absent - the client detects the language from the text | |
| `documentId` | UUID | name of the text |

The text is a root `Y.Text` named `code-mirror-<documentId>`, lines joined by `\n`; there is no entry in `documents`.

#### `flip-card`

| Field | Value | Meaning |
|---|---|---|
| `width`, `height` | 220×320 | |
| `scale` | 1 | |
| `side` | `front`, `back` | the key appears with the first flip; shared by both sides |
| `frontDocumentId`, `backDocumentId` | UUID | texts of the sides |

#### `dice`

| Field | Value | Meaning |
|---|---|---|
| `faces` | `2`, `4`, `6`, `8`, `10`, `12`, `20`, `"F"`, `"%"` | number of faces (`main@4068915`) |
| `value` | number | result |
| `spinTime` | ms | start of the throw |
| `rotation`, `scale` | `0`, `0.5` | |

#### `spinner-wheel`

| Field | Value | Meaning |
|---|---|---|
| `items` | `[{id, label}]` | a plain array; at most 20; default labels `Option <n>` |
| `mode` | `custom`; a workspace-members mode (value not captured) | |
| `userIds` | array | for the members mode |
| `removeItemOnceChosen` | bool | `false` by default |
| `excludedItemIds`, `selectedItemId`, `wheelRotation`, `spinTime` | `[]`, `null`, accumulated degrees, ms | spin state |
| `scale` | 1.5 | |

#### `phosphor-icon`

| Field | Value | Meaning |
|---|---|---|
| `iconName` | `anchor` | Phosphor icon name |
| `weight` | `thin`, `light`, `regular`, `bold`, `fill`, `duotone` | style; `regular` has no suffix in the address |
| `width`, `height` | 48×48 | |
| `fillColor` | `{color: "gray12", opacity: 1}` | |

#### `stamp`

| Field | Value | Meaning |
|---|---|---|
| `data` | `{type: "imageStamp", url: "Siba_Inu/1.png"}` or `{type: "textStamp", text: "🚀"}` | a sticker from a set, or an emoji |
| `width`, `height` | 130 (the long side is the set's `baseSize`) or 60 for an emoji | |

A relative `url` is `STATIC + "stickers/" + url`; an absolute one (`/assets/…`) is served by `app.holst.so`. Sets: `Siba_Inu`, `Offlinosaurs`, `Hands`, `Clipette` (`Skrepochka/`, `baseSize` 140), `Pixel Arts` (`Pixel_Art/`), `Wordie`, `Kite Kat`, `Blue Rondy` (the last two are absolute addresses).

#### `reaction-stamp`

| Field | Value | Meaning |
|---|---|---|
| `stampKey` | `like` (default), `heart`, `star`, `+1`, `dislike`, `figma-question`, `check`, `cross` | |
| `rotation` | degrees | random tilt |
| `width`, `height` | 60×60 | |
| `stickyPosition` | `undefined` | the key is there, the value is not; it does not attach to a frame or an object |

#### `drawing`

Pen and marker are one kind.

| Field | Value | Meaning |
|---|---|---|
| `algorithm` | `lazy` | smoothing |
| `path` | `"0,0,24.4,8.1,…"` | flat `x,y` pairs with a single sign, relative to `position` |
| `pivot` | `{x: 0, y: 0}` | |
| `strokeWidth` | 3 (pen), 22 (marker) | |
| `color` | integer RGB: `1973790` = `0x1E1E1E` | not a palette token |
| `alpha` | 1 (pen), 0.5 (marker) | |

#### `mind-map-node`

| Field | Value | Meaning |
|---|---|---|
| `documentId` | UUID | text |
| `nodeType` | `1` pill (default for the root), `2` rectangle, `3` rounded; absent - no background | shape |
| `nodeColor` | `{color, opacity}` | for a child - a random token of the root (`sky10`, `mint10` …) or the parent's colour; for the root `violet10`, 0.1 by default |
| `parentId` | parent | the tree is built through `parentId` |
| `layout`, `autoLayout` | `1` vertical/`2` horizontal (default), `true` | inherited from the root |

There is no size, it is computed.

#### `task-card`, `kanban`

| Entry | Fields | Meaning |
|---|---|---|
| object `task-card` | `titleDocumentId`, `descriptionDocumentId?`, `width` (300, at least 280), `scale?`, `index`, `statusId?`, `schedule?` `{start?, end? {date, time?}, timeZone?}`, `taskCardInfoId`, `color`, `displayOptions` | the title is a text in `documents`; no height |
| `color` | integer RGB: `11777204` (grey by default), `7327128`, `15584333`, `15836005`, `15233642`, `10457582`, `7118845`, `8569823`, `10274124`, `14317499` | not a token |
| `displayOptions` | `{statusLikeCheckbox?, fillBackground?}` | from the client's `localStorage` |
| `task-card-infoStore/<taskCardInfoId>` | `{id, tagIds, assigneeIds, statusId?, schedule?}` | tags, assignees (user ids), status, deadline |
| `task-card-statusesStore/<id>` | `{id, title}` | five default statuses: `backlog`, `to-do`, `in-progress`, `done`, `canceled` |
| object `kanban` | `kanbanInfoId`, `scale?`, `position` | no size and no text |
| `kanban-infoStore/<kanbanInfoId>` | `{id, columns: [id], swimlanes: [id]}` | order of columns and lanes |
| `kanban-columnsStore/<id>` | `{id, title, index}` | three columns: "To do" `0`, "In progress" `10000`, "Done" `20000` (the title is stored translated) |
| `kanban-swimlanesStore/<id>` | `{id, title, index}` | one lane "Untitled", `0` |
| a card in a column | `parentId` = the kanban, `swimlaneId`, `columnId`, `index`, width 338 | |

`index` between neighbours is a random number in the middle 80%, after the last one `+12345…22345`, for a sole one a random number in (-1e9, 1e9) (`main@5938625`).

#### `group`

The lock of an object is not a field but a new `group` object (`locked: true`, `ignoreZIndex: true`, `position` = the position of the object); the object's `parentId` is set to the group's `id`. Positions of the children stay absolute.

Bundle schemas (`main@5669586`): `image`, `document`, `arrow`, `drawing`, `link`, `frame`, `shape`, `simple-text`, `card`, `sticker`, `sticker-stack`, `group`, `task-card`, `kanban`, `stamp`, `file`, `table`, `table-cell`, `code`, `mind-map-node`, `flip-card`, `dice`, `reaction-stamp`, `spinner-wheel`, `phosphor-icon`. Kinds without their own variant in `ObjectKind` go into `ObjectKind::Other(name)` without loss, and such a name can be used as a filter (`--kind mind-map-node`) if the object is on the board; we cannot write them.

### Texts (rich text)

The text of an object is the entry `documents[<documentId>]`, a `Y.XmlText` in the slate-yjs layout: root → embedded `XmlText`s with attributes `type`, `key` (node uuid). Marks are boolean attributes on a leaf (`bold`, `italic`, `underline`, `strikethrough`; absent - off).

| Block | `type` | Attributes |
|---|---|---|
| paragraph | `paragraph` | |
| headings | `heading-one`, `heading-two`, `heading-three` | |
| bulleted, numbered list | `ul-list-item`, `ol-list-item` | the number is not stored; there is no container, items follow one another |
| checklist | `checkbox-list-item` | `checked` (for a card `false` is written at once, for a sticker only after it is ticked) |
| divider | `divider` | an empty node |
| code | `code-block` | `language` (`javascript` by default) |
| empty new document | `initial` | only `key` |
| inline link | `link` | `link` (URL; the key is exactly `link`), `key`; the leaf inside has `color: 1671390` (`0x1980DE`) |

Layout per kind. Sticker and text: one `wrapper` for the whole text, paragraphs inside. `card` document, note and cell: one `wrapper` per block, nesting is a `wrapper` inside the parent's `wrapper` after its block. Pasting a URL into editable text makes a link by itself (the link text is the URL); a URL typed by hand does not become a link. Editing a link replaces the node (a new `key`), removing it unwraps it into a leaf without attributes.

A link to an object is not text but the object's `linkTo` field. A table has no link button (pasting a URL into a cell still gives a link), nor does a `link` card and, apparently, `mind-map-node`; `flip-card` keeps the inline link in the document of the side that was edited.

### Tags

Definitions are the root map `task-card-tagsStore`: `<id>` → `{id, title, color}` (`color` is a token, for example `indigo5`). Renaming, changing the colour and deleting are edits of this entry, `/gapi` is not involved. An object carries `tagIds` (a sticker directly in the object, a task card in `task-card-infoStore`); the reader treats unknown ids as absent, because when a tag is deleted the client does not remove them.

### Board note

`board-note` is a root `Y.XmlText` of the board document (next to `objects`, not an entry of `documents`) in the card layout: one `wrapper` per block. The first element is a deleted `initial`. Writing the note uses the same writer as for a card, aimed at this root.

### Comments (`<board>$comments`)

Comments live not in the board document but in a separate room `<board>$comments` on the same `hud/ws` (roots `commentsStore` - a `Y.Map` - and `usersIds`). Messages are as for the board document: type 1 subscribe, type 3 contents (a sequence of chunks: the base, then each update), type 0 client write, type 2 acknowledgement. The room `<board>$state` holds the root `backgroundColor`. The socket `awareness/<board>` carries no comments. `GET /gapi/comment/thread_list/<board>` returns only read marks `[{channelId, readAt, threadId, userId}]`; records of deleted threads remain in it.

An entry in `commentsStore[<id>]` is a nested `Y.Map`, the id is 5 characters, an edit touches one key:

| Key | Thread | Reply | Meaning |
|---|---|---|---|
| `id` | yes | yes | the same as the key |
| `t` | yes | no | target: `{type: 2, point: {x, y}}` - a point in board coordinates; `{type: 1, targetId: <object>, point: {x, y}}` - fractions of the object's frame |
| `p` | no | yes | id of the thread the reply belongs to |
| `c` | yes | yes | body: `wrapper > paragraph > {text}`, block `key`s may repeat |
| `a` | yes | yes | author: the user's number (from the board's `usersIds`), not a uuid |
| `ca` | yes | yes | created, ms (fractional) |
| `m` | yes | yes | always `[]` (apparently mentions) |
| `r` | yes | no | resolved, `false` by default |
| `ra`, `rb` | resolved | no | when (ms) and by whom (author number); on reopening the keys are removed |
| `ro` | yes | no | `true` on all new threads, meaning unknown |
| `co` | with a colour | no | nested map `{color: "indigo6", opacity: 1}` |

A reply is a separate entry with `p` = the thread; the thread does not contain it. Deleting a thread removes its replies by `p` in the same transaction; the interface does not allow deleting a single reply. After a write the author sends `POST /gapi/comment/set_read_at {channelId, threadId, readAt}`; colour, resolving and reopening do not send it. The reader collects threads (those with `t`) and groups replies by `p`, sorting by `ca`.

### Palette and colours

A token is `<name><shade>` (`yellow4`, `gray12`, `red9`); `color: null` is transparent. Exceptions: `drawing.color` and `task-card.color` are integer RGB.

### Author number (`usersIds`)

Confirmed by the bundle (`UsersIdsService`) and live: the number is a 31-bit hash of the user's UUID (a cyrb53 variant: `0xdeadbeef ^ salt`, `0x41c6ce57 ^ salt`, `Math.imul` over UTF-16, result `& 0x7fffffff`), and `salt` grows from 0 while the number is taken by another user. For a given user id the computed number matched the entry on a live board. If we are not in `usersIds` (a new board), the web client and the official MCP add an entry in the same transaction; we do the same. Checked on a new board: after our write the official MCP shows `createdBy` = our user.

## Writing (stage 3)

Confirmed live on `zz-test-hud-write`:

- The client Update frame has no `encV2` marker, but the body is a Yjs update v2 (the provider listens to `updateV2`, the official MCP sends `mergeUpdatesV2`).
- `updateId` is a random integer below 2^52; the server answers with `Ack` carrying the same id. We report success only after Ack. Anything that breaks after the point of no return - sending the Update frame or an upload whose response may not have arrived (`OutcomeUnknown`) - is `HudUnconfirmed`: "update N (objects …) was sent but not confirmed — it may or may not have been applied", the cause (15 s timeout, socket closed or dropped, unreadable frame, refusal) is in the chain, the hint is to re-read the board (`holst object ls`, in MCP `object`) before retrying, otherwise a retry duplicates objects. MCP shows the same text and hint. An explicit upload refusal (HTTP status) is an ordinary error: the update was not sent.
- A change is built on the board as it was loaded: edits of other participants between loading and sending are not seen by us (the CRDT merges them, but, for example, an object that someone moved into a frame within that second is not carried along by our move of the frame).
- An Update larger than 4096 bytes: `POST /gapi/hud/upload?documentName=<board>&updateId=<id>` with Bearer and a body of `application/octet-stream`, then a Link frame: the size and the string `<board>/<id>`. Checked with a text of 6000 characters (update 14.7 KB).
- An Update is exactly the changes of one transaction, like the web client's (`updateV2` of a transaction): we apply it to the loaded `Doc`, `commit` and encode `TransactionMut::encode_update_v2` - the inserts of the transaction and only its deletes. The diff `encode_state_as_update_v2(state_vector_before)` does not fit: Yjs puts the entire delete set of the document into it, and on a large board (about 4400 objects, many deletions) a single sticker weighed 70 KB and went through upload. The bundle in `atomicCommitOnly` mode (the official MCP) also sends one transaction - one update. Import (`board import`) still sends the diff against the board's state vector: it needs everything that the server does not have.
- The ClientID of our `Doc` is 31-bit (yrs defaults to 53-bit; that worked too, but compatibility is safer). The web client takes a random uint32 and does not check it in advance; we re-pick the id while it is in the state vector of the loaded board (`change::client_id`), so that our blocks do not mix with other people's.
- Editing object fields is `set` on the keys of its Y.Map (`position`, `width`/`height`, `fillColor` or an arrow's `strokeColor`, a frame's `labelText`) plus `updated {a, t}`.

### Frames and `parentId`

As the web client does (by the bundle): when dragging, the parent is the topmost frame by rendering under the cursor (a nested one above the outer one, among siblings the larger `zIndex`), the record is `{parentId: id}`; taking an object out of all frames writes `parentId: null` (the key is not removed). Frames nest inside each other. A dropped frame takes the siblings (same parent) that lie entirely inside it. Dragging a frame moves its whole subtree by `position` and the arrows with both ends in the subtree; for an arrow the free ends (`start/end.point` without `objectId`) and the ends on moving objects are shifted, as are `elbowMidwayControls`/`midwayControls`; an arrow's `position` never changes. A new object goes into the frame under the click point; an arrow with two attached ends goes into the deepest common frame of the ends.

We have no cursor, so "under the cursor" becomes "entirely inside" (for a text without width - the corner):

- `object edit --at`: `parentId` is the deepest frame (with equal depth, the one with the larger `zIndex`) that entirely contains the object at its new place, other than the object itself and its contents; outside frames - `null`. It is written only if the parent changed (the `parent` field in the output).
- Moving a frame: the subtree and arrows as in the web client above; then the frame takes the siblings that ended up entirely inside.
- `object add` without `--frame`: the same frame choice for the new object; an arrow goes into the deepest common frame of the ends.
- `object add --frame <frame>`: the object must lie entirely in the frame (`--at` is from its corner), otherwise `OutsideFrame`; the parent is the deepest frame inside the given one that contains the object.
- Gap: the web client recomputes an arrow's `parentId` (the common frame of the ends) on any movement of its ends; we do so only when the arrow is created, so after an arrow end is moved into another frame it keeps its old `parentId` (this does not affect the drawing: the client computes attached ends itself).

Checked live on `zz-test-review-fix`: a sticker in a frame got its `parentId`, `--frame` at a point of a nested frame gave the nested one, an arrow between children gave the common frame; taking a sticker out wrote `null`, moving it in wrote the new frame; moving a frame by +1000 in x carried the nested frame and its stickers, of an arrow only the end on a moving object shifted, `position` stayed (0, 0), and a sticker that ended up entirely inside became its content; `--frame` past the edge of a frame gave `OutsideFrame`.
- Editing text: the contents of the existing XmlText are cleared and written anew (one wrapper per line); if there is no document, a new one is created and `documentId` is written into the object. The web editor also overwrites the text this way (`replaceFromJSON`).

### Deletion

- Deletion is hard: the object key is removed from `objects` (`objectsMap.delete(id)`), there is no "deleted" flag. The web client does not delete the entry in `documents` (the editor's `removeEditor` is empty) - we leave it too.
- The web client deletes the subtree: a frame together with the objects whose `parentId` it is. An arrow whose end was deleted is deleted by any open client (`initListenerDataServiceDeleteEndObjects`) - without an open client it would dangle, so we delete such arrows ourselves in the same update.
- Every deletion writes an entry into the journal `delete-events-metaStore` (below) in the same transaction as the deletion, one entry for everything deleted (the object, the frame subtree, arrows). The web client sends the entry in a separate transaction after 250 ms (the `onObjectDeleted$` buffer); we put it in the same update; the result is not affected.

### Deletion journal (`delete-events-metaStore`)

Analysed from the bundle `main.5b1ea1b9.js` (classes `x8` - the shared journal, `x9` - the local one, `je` - writing and rollback, `x7` - the panel) and checked against the structure of 961 entries on a read-only copy of a large board, without contents.

Why we need it: a deletion in Yjs is only a delete set, it does not move the state vector. A cache whose state vector covers the server's skips the snapshot link and does not see deletions. A journal entry is an insert, it moves the state vector, and the cache fetches the board again. The web client writes it on every deletion, so its deletions are visible; without the entry our deletions would be visible neither to other holst-rs caches nor to the «Активность» (Activity) panel.

Storage: the root is a Y.Map "entry id → Y.Map of fields". The name is `<storeName>Store` from `new YStore("delete-events-meta")`, the backend is the same `YMapService` as for `objects` (`_add`: each field is a `set` in the entry's Y.Map; an array of non-objects would become a Y.Array, but `s` is an array of objects and is stored as a plain JSON value). All fields are plain values (`ContentAny`), there are no Y types inside.

| Field | Type | Meaning |
|---|---|---|
| `id` | string of up to 5 characters | entry id, also the key in the root: a hash of the sorted unique ids of the deleted objects - `r = (31·r + charCode) mod (2^31 − 1)` over their concatenation, `toString(36)`, first 5 characters (for `["a"]` - `2p`, for an empty set - `0`). Deleting the same set again overwrites the entry (`upsert`) |
| `a` | number | author number from `usersIds` (`currentUserNumberId`) |
| `t` | number | deletion time, Unix ms; the panel uses it to find the entry on rollback (`getEventFromTimestamp`) |
| `cnt` | number | how many objects were deleted (the length of `s`) |
| `s` | array or `null` | full fields of the deleted objects as they were in `objects` (`removedState`); `null` - snapshots were trimmed |
| `type` | string, optional | the type of the objects if it is the same for all (`sticker`, `frame`, …); with mixed types the field is absent |
| `unlock` | `true`, optional | only for a deleted group (`type: "group"`) with locked objects |

On the copy of the large board: all 961 entries have `id` (equal to the key), `a`, `t`, `cnt`, `s`; `type` - 767 of them; `unlock` - none; 852 have `s: null`, for the other 109 `cnt` equals the length of `s`, and the hash of the object ids from `s` matched the key in all 109. The states in `s` are ordinary object fields (`id`, `type`, `position`, `zIndex`, `created`, `updated`, `parentId`, `documentId`, `fillColor`, …).

Cleanup is on the web client side when a board is opened: at most 1000 entries (older ones are deleted, `cutEventsCount`), and for entries older than a day, starting from the 500th deleted object, `s` is replaced with `null` (`cutDeletedObjectsCount`). The local copy (`<board>$local-deleted-objects` in IndexedDB, `{id, t, states}`, 90 days) exists only for the one who deleted and does not get into the board's Y.Doc.

Restoring is entirely on the client (`rollbackDeleteActivity`): by `t` the states are taken (first from the local copy, otherwise `s` of the shared entry), sorted by `zIndex` and inserted back into `objects` under the same ids (`addRestoredObject` → `objects.set(id, Y.Map(state))`); an arrow only if both its ends exist, for an object without a parent `parentId` becomes `null`. Texts in `documents` are not touched on deletion, so they return together with the object. After the insert the client deletes the journal entries with these ids (`removeEventsByBoardObjectIds`). An entry with `s: null` cannot be restored (`cant-be-restored`).

The panel: the board menu (the ☰ button at the top left, next to the name) → «Активность» (Activity, `board.burger-menu.activity`) opens the left «История» (History) panel on the «Активность» tab: «Удалено» (Deleted, `cnt` 1) or «Удалены» (Deleted, `cnt` > 1), the author by `a`, the time by `t`, a restore button. From the browser console the same panel is opened by `window.__boardPanelHistory__()`.

What we write (`change/journal.rs`, in the same transaction as the deletion): `{id, a, t, cnt, s, type?, unlock?}` exactly as above; `s` is the fields of the deleted objects from the loaded document, `a` is our number (added to `usersIds` in the same update if needed), `t` is the time the change was built. This is how `holst object rm`, MCP `object_write` with `action: rm` and the cascades (frame contents, attached arrows - one entry) write. Tests: the id matches the JS hash, the entry matches the web client's sample of the shape, the update moves the state vector (while a bare deletion does not), a rollback by the entry in the same way as the client's returns the objects with fields and texts.

Checked live on `zz-test-deljournal` (easy, then deleted permanently): two stickers, a frame with a nested sticker and a ballast sticker; we deleted a sticker and the frame (`--yes`). Re-reading - two stickers remained; two entries appeared in the server document: `{a, cnt: 1, id, s: [sticker], t, type: "sticker"}` and `{a, cnt: 2, id, s: [frame, sticker], t}` (without `type`); the server state vector grew by two new clients (5 → 7); a second holst-rs cache (a separate `HOLST_CACHE_DIR`, seeded before the deletion) saw the deletion. The official MCP on `localhost:47641` was not running at that moment.

Cache safety net: even with the deletion journal, deletions by clients that do not write it could be lost. Therefore the board cache (`command/board_load/`) trusts a copy only for `HOLST_CACHE_TRUST` (`24h` by default; `30m`, `7d`, a number - seconds; `0` - always download) after the last full load (`fetched_at` in `meta.json` is now the time of the last full load: without a seed or with a downloaded link; a state vector check without a link does not move it). Older - we load without a seed, in full; the same for the index and for writing. `--offline` still serves an old copy. The MCP server's memory behaves the same way: a board downloaded in full longer ago than this period does not serve as a seed.

## Area snapshots (stage 4)

There is no server-side export of a board or an area to PNG/SVG/PDF: neither in the REST specification, nor in the bundle `main.5b1ea1b9.js`, nor in the desktop `app.asar`. The web client draws everything itself with the Skia/CanvasKit engine (WASM) and makes the SVG/PDF/PNG export from it (`exportService`). The board preview is client-side too: `generateBoardPreview(800, 800)` → `POST /gapi/upload_file` → `PATCH /gapi/channel/{id}` `{thumbnail}`; to read - `https://storage-edge.app.holst.so/storage/<thumbnail>`, updated about once every 10 minutes while the board is open with write access. The official MCP draws snapshots in the desktop application itself (`get_screenshot` on `127.0.0.1:47641`).

So the renderer is our own and pure: `domain/area/` (object frames, arrow routes, area selection), `domain/text_layout.rs` (Inter text width, wrapping, sticker font fitting), `domain/palette.rs` (tokens → colours), `domain/picture/` (SVG), `domain/geometry.rs` (rectangles). PNG is `system/raster.rs` on resvg behind the `png` feature.

- Palette: Radix Colors (light theme, the old HSL) with the board's corrections: `white1..12` = `#ffffff`, `gray8` = `#b3b3b3`, `gray11` = `#171717`, `yellow8` = `#ffe366`. `color: null` is transparent; a number instead of a token is RGB. Text on a fill: `gray12` if `(299R+587G+114B)/1000 > 160`, otherwise white.
- Sticker: size `width×height × textScale`, no rounding, a shadow; text Inter, line spacing 1.25, padding 16; the font size is the largest of 4…64 in steps of 2 at which the text fits and a word of up to 16 characters is not broken (`fontSize` is the ceiling).
- Simple text: 14 px × `textScale`, line `round(1.5 × font size)`, wrap at 600 (or `width` with `fixedWidth`).
- Shape: `shapeType` is about 60 kinds (basic, flowchart, BPMN); we draw rectangles, ellipses, polygons, a speech bubble, a document, a cylinder, and the rest as a rounded rectangle. Text 16 px (`fontSize`), line 1.5, padding 12.
- Frame: rounding `min(6, w/8, h/8)`, stroke 1.1; the label is a plate 24 high above the frame (y − 30), Inter 13/500; the label and stroke colours follow the client's rules from the fill token.
- Arrow: `start`/`end` = `{point, objectId?, relativePoint?, control? (1 top, 2 right, 3 bottom, 4 left)}`, `arrowType` straight/elbow/curved, `arrowheadStart` (`none`) / `arrowheadEnd` (`miro`). Without `control` and `relativePoint` the ends are the midpoints of the nearest sides. The polyline is built in a simplified way: an exit of 32 from the object, one or two turns, corners rounded with a radius of up to 20 (the client searches for a path with Dijkstra over the corners of expanded frames - on complex layouts ours can differ).
- Image: `STATIC + name`, where `STATIC` = `https://storage-edge.<host>/storage/`; crop `cropTransform`; the original name is `originalName`, the type is `imageType`. The `file` object: `fileName` (name in storage), `displayFileName`, `fileSize`, `fileType`, without sizes (`scale`). Storage serves the file as is (the `Content-Type` header can be wrong: a PNG arrives as `image/webp`), so the format is determined from the first bytes. In SVG - an `<image>` with the address over a grey placeholder (`--embed` - a `data:` URL); in PNG - the image itself: addresses from the snapshot are downloaded without token and cookies only from `STATIC`, cached in `assets/` of the account cache, resvg gets the bytes through its own resolver and does not go to the network or files itself; if the download failed - the placeholder.
- Table: rows and columns are keys `row-<id>`/`column-<id>` `{index, height|width}` on the table, cells are `table-cell` objects with `rowId`/`columnId`.
- Layer order: by `zIndex` among siblings, the contents of a frame are above the frame (the key is the chain of `zIndex` from the root).
- Area: a frame, objects with fields, a rectangle `x,y,w,h` or the whole board. A frame is looked up in order: exact id or link → exact title (case- and space-insensitive) → unambiguous id prefix → unambiguous part of a title; `--frame 2` is the frame titled "2", not the frame whose id starts with 2. Several frames with one title are an ambiguity with a list. First a filter by object frames (a whole large board, 4402 objects, takes about 20 ms for SVG, a frame of 768 objects 4 ms; the sticker font is found by binary search over sizes, the lines are only counted meanwhile: at a smaller font greedy wrapping does not give more lines), only what intersects the area is drawn; `holst object ls --rectangle/--frame` shows exactly these objects.
- Corrupt board data: an object with non-numeric, infinite or larger than 10¹² in absolute value coordinates and sizes (including after `textScale`) does not enter the area and is not drawn. An arrow whose end refers to a deleted object, a group, an arrow, or a cell without a table is not drawn and does not enter the area (otherwise the end went to (0,0) and stretched the area to the whole board). A free end without `objectId` follows the saved point.
- Snapshot size: `--max-side` (16…16384, 1568 by default in MCP (small text is readable, ~2400 image tokens for a frame), 8192 in the CLI) is strictly observed, the scale is lowered as much as needed; at most 4096×4096 pixels in total (64 MB for rendering), a long narrow picture can reach 16384 along a side. `--scale` is greater than 0 and at most 100, `--padding` is 0…100000; out of range - a `SnapshotOption` error before the board is loaded. `Picture.scale` is the scale it was drawn at. If it came out below 0.1 pixel per board unit, the CLI and MCP warn: the text is unreadable, a far object stretched the area - better a frame, objects or a smaller area.
- Text in SVG: characters forbidden in XML (control characters other than tab and line feeds, U+FFFE, U+FFFF) are dropped - resvg otherwise failed with `NonXmlChar`. Vertical tab and form feed (soft breaks from PowerPoint), U+2028 and U+2029 are a line break, like `\n`.
- Terminal: board text in the CLI's text output (`object ls`, `board show`, errors with frame titles and ids) goes through `text::printable` - ESC, other C0/C1, DEL and the bidi controls U+202A–U+202E, U+2066–U+2069 become `�`. JSON stays exact (serde escapes control characters). SVG is not printed to a terminal (`SvgToTerminal`, like PNG): it carries board text, and cleaning it would spoil the picture; a file or a redirect is needed. A `--format` that contradicts the file extension (`x.svg --format png`) is an error.

## Links

- An object in the browser: `https://app.holst.so/board/{board}?objectId={object}` (`BaseUrl::object_link`; ids are encoded `%XX`, except for the unreserved characters of RFC 3986); the client itself builds `…/share/b/{board}?objectId=…`, the route `/share/b/` redirects to `/board/` with the same query.
- Desktop: `/open-in-desktop?boardId={board}&objectId={object}` (`BaseUrl::desktop_link`), inside the client - `holst://open?url=<https-link>`.
- The board reads no other parameters: no x/y/zoom, no several objects, no `frameId`. A link to an area is a link to the smallest frame that contains it, otherwise to the object nearest the centre (`Scene::anchor`). For a snapshot and a list of a selected frame the link is to the frame itself (`Scene::link_target`): the frame's area is wider than the frame because of the label and padding and would fall into the outer frame.

## What the code has

- CLI: `holst object ls <board> [-w ws] [-o json|text]` - kind, place, size, colour, text, link; `-o json` gives full ids. Filters `-k`, `--contains`, `--tag`, `--offset`, `-l`.
- CLI writing: `holst object add sticker|text|shape|frame|arrow|link|image|file <board> …`, `holst object edit <board> --object <object> [--text] [--at x,y] [--size WxH] [-c token]`, `holst object rm <board> --object <object>` (a frame with its objects and several objects - only with `-y`). An object is a full id, a link with `objectId=`, `@N` in MCP, or an unambiguous start of an id of 8 or more hexadecimal characters (the first UUID group); shorter, non-hex (a title like `Cafe`) or word-like input is an error with a list of ways, not a random object. The same for `--frame`, `--from`/`--to` and in MCP. Every command has `-n, --dry-run`: the board is loaded, the update is built, what will change is printed; nothing is sent. The output is the id and link of every created and changed object.
- CLI snapshots and areas: `holst board snap <board> [FILE] [--frame F | --object O… | --rectangle x,y,w,h] [--padding N] [--format png|svg] [--scale N] [--max-side PX]`; the same selection in `board show`, `board save` and `object ls`; `object add … --frame <frame>` (then `--at` is from the frame's corner, and `parentId` is written into the object). A link to a frame or an object in place of `<board>` selects that part: a frame - like `--frame`, another object - like `--object`, without `objectId` - the whole board; `snap` and `save` write to `~/Downloads` by default without overwriting.
- MCP: `board` with `action: snap` (a PNG image + a line with the area, the object count and a link; without the `png` feature - SVG as text); `object` with area selection (`rectangle`, `frame`, `object`, `padding`), filters `kind`/`contains`/`tag` and pages `offset`/`limit`; `object_write` has `frame`.
- CLI board copy (`command/board_file.rs`, the pure part is `domain/board_file/document.rs`): `holst board export <board> [FILE] [--uncompressed] [--object-list]` - the whole document as one Yjs update v2 (`encode_state_as_update_v2` from an empty state vector, with deletions) in gzip (`domain/board_file/compression.rs`, miniz_oxide; `.gz` is appended to a name without it), with `--uncompressed` as is; with `--object-list` next to it `<file without .gz>.objects.jsonl` - one object per line, like `object ls -o json`, without links. Measured on a large board (4402 objects): update 4.6 MB, gzip 1.42 MB (3.2 times smaller, 70 ms), zstd -9 would give 1.09 MB, but it is a C library and +0.45 MB of binary; rebuilding the document (`encode_state_as_update_v2` of a fresh `Doc`) and GC do not change the size - deleted content is already collected. `board import` and `backup get` (also gzip by default) recognise gzip by the first bytes `1f 8b`, unpack at most 1 GiB and check the CRC; `holst board import <file>` creates a new board (`-w`, `--folder`, `--name`); `--into <board>` writes the file into an existing one - the diff against the loaded board, through upload - and requires `-y` (`--merge` keeps what is on the board). `--into` takes only an id, a link or the full board name (case and punctuation do not matter), a part of the name does not fit: a fuzzy match would merge the copy into someone else's shared board. Importing an export back into the same board restores nothing: in a CRDT deletions are stronger than old inserts, deleted objects stay deleted - this is not a rollback method; roll back onto a new empty board or through server backups. Checked live: a board of 4402 objects, an update of 4.6 MB through upload, Ack arrived; the server snapshot `backup get` is the same v2 format and can also be imported. Images refer to the shared `storage-edge.app.holst.so/storage/<name>` and are not copied; comments (`<board>$comments`) are not carried over.
- MCP: `object` (read only); `object_write` - `action` (`add` with `kind` `sticker`, `text`, `shape`, `frame`, `arrow`, `link`, `image`, `file`; `edit`, `cp`, `rm`, `get`) and `dry_run`; an object can be given as `@N` from the `object` response. `rm` writes a deletion journal entry, like the CLI.
- When reading, exactly one message leaves us - `RequestDocument`; a test on a fake socket checks this. With `--dry-run` also only that one.

## Risks

- The protocol is not public: the frame format, field names and `atomicCommitOnly` may change without notice. An unfamiliar board frame type is `MalformedFrame`, not silent; the length of such a frame is unknown, so the rest of the WS message is not read. An unfamiliar frame of another document is skipped (as the web client does, the rest of the message is lost) with a log entry.
- A wrong update on write will corrupt the board for all participants (the CRDT sends it to everyone); write only to zz-test boards and verify by reading.
- The `readOnly` response is the permission in the server's opinion: with `readOnly: true` we neither build nor send an update (`BoardReadOnly`). Guest permissions (as on a shared team board) have not been checked.
- A guess: the server apparently does not check the shape of objects - wrong fields will be distributed to all clients; therefore the fields of new objects are copied from objects of the official MCP, not invented.
- Large boards: the whole document is loaded into memory; the load timeout is 30 s.
- Weight of dependencies: `yrs` and `tokio-tungstenite` (rustls, ring) added 18 packages and about 0.7 MB to every binary. SVG rendering is about 0.2 MB with no new packages; PNG (`resvg` with `text`, `system-fonts` and `raster-images` - PNG/JPEG/GIF/WebP decoders for images on the board, without SVGZ and mmap) is on by default in both the CLI and `holst-mcp`: `holst` 5.9 → 8.7 MB (resvg about 2.1 MB, image decoders about 0.5 MB), `holst-mcp` 8.7 → 9.2 MB; `--no-default-features` of `holst-cli` leaves only SVG. `url` is built without Unicode IDNA (`idna_adapter` pinned at 1.0.0): minus 22 ICU4X packages and about 160 KB in each binary; a non-ASCII host is given in punycode (`xn--…`). Code that runs once per process or request (`clap_builder`, `rmcp`, `schemars`) is built with `opt-level = "s"`.
- A PNG snapshot uses system fonts: Inter is usually not installed, so the text is set in a fallback font (Helvetica Neue/Arial), and line breaks may differ slightly from the client. The font files of the families that the SVG names are read once and kept in memory (about 9 MB on macOS): without this resvg re-read the font file from disk on every access, and the PNG of a large board's frame was drawn in 1 s instead of 0.14 s. We estimate text width from the Inter width table, without shaping.
