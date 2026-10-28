// What the holst-mcp server costs a model: bytes and tokens of the server
// instructions, tools/list and typical tool replies, over stdio as a client
// sees them. Images count as width*height/750 tokens.
//
// Tokens: @anthropic-ai/tokenizer when TOKENIZER points at a directory where
// it is installed (`npm i @anthropic-ai/tokenizer` there), else chars/3.2.
//
//   node crates/holst-mcp/scripts/measure.mjs                 # no login: temp HOLST_HOME
//   MEASURE_LIVE=1 node crates/holst-mcp/scripts/measure.mjs  # the user's login, reads only
//
// Env: HOLST_MCP_BIN (default target/debug/holst-mcp), MEASURE_FRAME (a frame
// title for the live frame calls), MEASURE_CALLS (a JSON
// file [[label, tool, args], ...] replacing the default calls), MEASURE_JSON=1
// prints the rows as JSON, MEASURE_SHOW=N prints each reply's first N chars to
// stderr, MEASURE_SAVE=dir saves the images.

import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL("../../..", import.meta.url));
const BIN = process.env.HOLST_MCP_BIN ?? join(ROOT, "target/debug/holst-mcp");
const LIVE = process.env.MEASURE_LIVE === "1";

const countTokens = (() => {
  try {
    const require = createRequire(join(process.env.TOKENIZER ?? "/nonexistent", "x.js"));
    const { countTokens } = require("@anthropic-ai/tokenizer");
    return { method: "@anthropic-ai/tokenizer", count: countTokens };
  } catch {
    return { method: "chars/3.2", count: (text) => Math.ceil([...text].length / 3.2) };
  }
})();

// A large board, its workspace and a copy of it; reads only.
const GENERAL = process.env.MEASURE_BOARD;
const WS = process.env.MEASURE_WS;
const EASY = process.env.MEASURE_COPY;
if (LIVE && !(GENERAL && WS && EASY)) {
  console.error("usage: MEASURE_LIVE=1 MEASURE_WS=<workspace id> MEASURE_BOARD=<board id> MEASURE_COPY=<copy board id> node measure.mjs");
  process.exit(2);
}
// A frame of the board to list and draw, by title; without it these calls are skipped.
const FRAME = process.env.MEASURE_FRAME;

const LIVE_CALLS = [
  ["status", "auth", {}],
  ["list workspaces", "workspace", { action: "ls" }],
  ["board ls tree", "board", { action: "ls", view: "tree", workspace: WS }],
  ["board ls trash", "board", { action: "ls", view: "trash", workspace: WS }],
  ["board ls favorites", "board", { action: "ls", view: "favorites", workspace: WS }],
  ["search", "search", { query: "general" }],
  ["board show", "board", { action: "show", board: GENERAL }],
  ["object 4400", "object", { board: GENERAL }],
  ["object frame", "object", { board: GENERAL, frame: FRAME }],
  ["object copy", "object", { board: EASY }],
  ["board snap frame", "board", { action: "snap", board: GENERAL, frame: FRAME }],
  ["board snap whole copy", "board", { action: "snap", board: EASY }],
  ["object_write dry_run", "object_write", { action: "add", board: EASY, kind: "sticker", text: "probe", dry_run: true }],
  ["error: unknown board", "board", { action: "show", board: "no-such-board-xyz" }],
  ["error: unknown alias", "object", { board: "@99999" }],
];

const OFFLINE_CALLS = [
  ["status", "auth", {}],
  ["error: no login", "workspace", { action: "ls" }],
  ["error: revoke unconfirmed", "board_write", { action: "revoke", board: "x", user: "x" }],
];

const calls = process.env.MEASURE_CALLS
  ? JSON.parse(readFileSync(process.env.MEASURE_CALLS, "utf8"))
  : LIVE ? LIVE_CALLS.filter(([, , args]) => !("frame" in args) || FRAME) : OFFLINE_CALLS;

const env = { ...process.env };
if (!LIVE) env.HOLST_HOME = mkdtempSync(join(tmpdir(), "holst-measure-"));

const server = spawn(BIN, [], { env, stdio: ["pipe", "pipe", "inherit"] });
const waiting = new Map();
let buffer = "";
server.stdout.setEncoding("utf8");
server.stdout.on("data", (chunk) => {
  buffer += chunk;
  let at;
  while ((at = buffer.indexOf("\n")) >= 0) {
    const line = buffer.slice(0, at);
    buffer = buffer.slice(at + 1);
    if (!line.trim()) continue;
    const message = JSON.parse(line);
    waiting.get(message.id)?.(message);
    waiting.delete(message.id);
  }
});

let next = 0;
const send = (message) => server.stdin.write(JSON.stringify(message) + "\n");
const request = (method, params) =>
  new Promise((done) => {
    next += 1;
    waiting.set(next, done);
    send({ jsonrpc: "2.0", id: next, method, params });
  });

// Width and height from a base64 PNG's IHDR.
const pngSize = (data) => {
  const bytes = Buffer.from(data, "base64");
  return [bytes.readUInt32BE(16), bytes.readUInt32BE(20)];
};

const rows = [];
const row = (label, text, imageTokens = 0, note = "") =>
  rows.push({
    label,
    bytes: Buffer.byteLength(text, "utf8"),
    tokens: countTokens.count(text),
    imageTokens,
    note,
  });

const init = await request("initialize", {
  protocolVersion: "2025-06-18",
  capabilities: {},
  clientInfo: { name: "measure", version: "0" },
});
send({ jsonrpc: "2.0", method: "notifications/initialized" });
row("instructions", init.result.instructions ?? "");
const tools = await request("tools/list", {});
row("tools/list", JSON.stringify(tools.result.tools), 0, `${tools.result.tools.length} tools`);
// What a model reads of a tool: name, description and input schema; annotations go to the client.
const visible = JSON.stringify(tools.result.tools.map(({ name, description, inputSchema }) => ({ name, description, inputSchema })));
row("tools/list, model-visible", visible);
row("instructions + tools/list", (init.result.instructions ?? "") + JSON.stringify(tools.result.tools));

for (const [label, name, args] of calls) {
  const reply = await request("tools/call", { name, arguments: args });
  const content = reply.result?.content ?? [];
  const text = content.filter((b) => b.type === "text").map((b) => b.text).join("");
  const images = content.filter((b) => b.type === "image").map((b) => pngSize(b.data));
  content
    .filter((b) => b.type === "image" && process.env.MEASURE_SAVE)
    .forEach((b, i) => writeFileSync(join(process.env.MEASURE_SAVE, `${label.replace(/\W+/g, "-")}-${i}.png`), Buffer.from(b.data, "base64")));
  const imageTokens = images.reduce((sum, [w, h]) => sum + Math.ceil((w * h) / 750), 0);
  const size = images.map(([w, h]) => `${w}x${h}`).join(" ");
  const error = reply.result?.isError ? "error" : reply.error ? `rpc ${reply.error.message}` : "";
  row(label, text, imageTokens, [size, error, `${text.split("\n").length} lines`].filter(Boolean).join(" "));
  if (process.env.MEASURE_SHOW) process.stderr.write(`--- ${label}\n${text.slice(0, Number(process.env.MEASURE_SHOW))}\n`);
}
server.stdin.end();
server.kill();

if (process.env.MEASURE_JSON === "1") {
  console.log(JSON.stringify({ method: countTokens.method, rows }, null, 1));
} else {
  console.log(`tokens: ${countTokens.method}; images: w*h/750`);
  console.log("| what | bytes | tokens | image tokens | note |");
  console.log("|---|---:|---:|---:|---|");
  rows.forEach((r) => console.log(`| ${r.label} | ${r.bytes} | ${r.tokens} | ${r.imageTokens || ""} | ${r.note} |`));
}
