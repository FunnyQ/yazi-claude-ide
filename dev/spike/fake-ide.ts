#!/usr/bin/env bun
// protocol-spike: a minimal fake IDE, built to answer one question — if we hand
// Claude only a file path and no file contents, does the file reach its context?
// Usage: bun spike/fake-ide.ts [workspace-dir...]
import { randomBytes } from "node:crypto";
import fs from "node:fs";
import { homedir } from "node:os";
import path from "node:path";

const SPIKE_DIR = import.meta.dir;
const STATE_FILE = path.join(SPIKE_DIR, "state.json");
const IDE_DIR = path.join(
  process.env.CLAUDE_CONFIG_DIR || path.join(homedir(), ".claude"),
  "ide",
);

const authToken = randomBytes(16).toString("hex");
// Several dirs may be passed so the workspace-policy spike can test whether
// Claude matches any entry in workspaceFolders or only the first.
const workspaces =
  process.argv.length > 2
    ? process.argv.slice(2).map((d) => path.resolve(d))
    : [process.cwd()];

type Selection = {
  start: { line: number; character: number };
  end: { line: number; character: number };
  isEmpty: boolean;
};

type State = {
  filePath?: string | null;
  text?: string;
  selection?: Selection;
  /**
   * The marked-files spike's independent variable. `selection_changed` is a
   * single-slot state — pushing it N times leaves only the last path — so a set
   * of marked files needs `at_mentioned`, one notification per file. Documented
   * in PROTOCOL.md, never measured against a real CLI.
   */
  mentions?: string[];
  /**
   * When set, every mention carries this `[lineStart, lineEnd]`. When absent the
   * two fields are omitted entirely. Measured 2026-08-08: sending `0, 0` to mean
   * "the whole file" renders as `@PLAN.md#L1`, so the CLI reads 0 as a line
   * anchor rather than as "no range". Omitting is the other thing to try.
   */
  mentionRange?: [number, number];
};

type SelectionPayload =
  | { success: false; message: string }
  | { success: true; filePath: string; text: string; selection: Selection };

type RpcMessage = {
  jsonrpc: "2.0";
  id?: string | number;
  method?: string;
  params?: Record<string, unknown>;
  result?: unknown;
  error?: { code: number; message: string };
};

let logStream: fs.WriteStream | null = null;

const START = Date.now();

function log(dir: "in" | "out", msg: unknown): void {
  // `t` is milliseconds since startup. The openDiff experiment measures how long
  // the CLI waits for an answer, and that is unreadable from an untimed log.
  const line = JSON.stringify({ t: Date.now() - START, dir, msg })
    .replaceAll(authToken, "<TOKEN>")
    .replaceAll(homedir(), "~");
  logStream?.write(line + "\n");
  console.error(`${dir === "in" ? "←" : "→"} ${line.slice(0, 300)}`);
}

function readState(): State {
  try {
    return JSON.parse(fs.readFileSync(STATE_FILE, "utf8")) as State;
  } catch {
    return { filePath: null };
  }
}

function selectionPayload(): SelectionPayload {
  const state = readState();
  if (!state.filePath)
    return { success: false, message: "No active editor found" };
  // This is the spike's independent variable: `text` is deliberately left empty
  // so that the path is the only thing we hand over.
  return {
    success: true,
    filePath: state.filePath,
    text: state.text ?? "",
    selection: state.selection ?? {
      start: { line: 0, character: 0 },
      end: { line: 0, character: 0 },
      isEmpty: true,
    },
  };
}

const TOOLS = [
  {
    name: "getCurrentSelection",
    description: "Get the current selection in the active editor",
    inputSchema: { type: "object", properties: {} },
  },
  {
    name: "getLatestSelection",
    description: "Get the most recent selection",
    inputSchema: { type: "object", properties: {} },
  },
  {
    name: "getOpenEditors",
    description: "Get information about currently open editors",
    inputSchema: { type: "object", properties: {} },
  },
  {
    name: "getWorkspaceFolders",
    description: "Get all workspace folders currently open in the IDE",
    inputSchema: { type: "object", properties: {} },
  },
  // The tools below are outside the MVP scope. They were added to test the
  // hypothesis that the CLI needs a complete tool set before it adopts a
  // connection as the active IDE. That hypothesis turned out to be WRONG —
  // adoption depends on the workspace matching, not on the tool count. They stay
  // because Claude does call tools that were never advertised.
  {
    name: "getDiagnostics",
    description: "Get language diagnostics",
    inputSchema: {
      type: "object",
      properties: { uri: { type: "string" } },
    },
  },
  {
    name: "openFile",
    description: "Open a file in the editor",
    inputSchema: {
      type: "object",
      properties: {
        filePath: { type: "string" },
        preview: { type: "boolean" },
        startText: { type: "string" },
        endText: { type: "string" },
        selectToEndOfLine: { type: "boolean" },
        makeFrontmost: { type: "boolean" },
      },
      required: ["filePath"],
    },
  },
  {
    name: "openDiff",
    description: "Open a diff view for the file",
    inputSchema: {
      type: "object",
      properties: {
        old_file_path: { type: "string" },
        new_file_path: { type: "string" },
        new_file_contents: { type: "string" },
        tab_name: { type: "string" },
      },
      required: [
        "old_file_path",
        "new_file_path",
        "new_file_contents",
        "tab_name",
      ],
    },
  },
  {
    name: "checkDocumentDirty",
    description: "Check if a document has unsaved changes",
    inputSchema: {
      type: "object",
      properties: { filePath: { type: "string" } },
      required: ["filePath"],
    },
  },
  {
    name: "saveDocument",
    description: "Save a document",
    inputSchema: {
      type: "object",
      properties: { filePath: { type: "string" } },
      required: ["filePath"],
    },
  },
  {
    name: "close_tab",
    description: "Close a tab by name",
    inputSchema: {
      type: "object",
      properties: { tab_name: { type: "string" } },
      required: ["tab_name"],
    },
  },
  {
    name: "closeAllDiffTabs",
    description: "Close all diff tabs in the editor",
    inputSchema: { type: "object", properties: {} },
  },
];

// Isolation experiment: with SPIKE_MINIMAL_TOOLS=1, advertise only the first
// four read-only tools. `tools/call` behaviour is untouched — the only variable
// is what `tools/list` returns.
const ADVERTISED = process.env.SPIKE_MINIMAL_TOOLS ? TOOLS.slice(0, 4) : TOOLS;

const asText = (obj: unknown) => ({
  content: [{ type: "text", text: JSON.stringify(obj) }],
});

/**
 * The openDiff experiment. F5 answers `-32601`, and the CLI still prints
 * `Opened changes in yazi` in place of its inline diff, so the user approves a
 * change nothing showed them. Opening a real diff in yazi's pane needs three
 * facts this switch exists to collect:
 *
 * 1. `accept` — does the CLI keep its own confirmation prompt after
 *    `DIFF_ACCEPTED`, or does the answer become the only veto?
 * 2. `hang` — how long does the CLI wait, and does it block while waiting?
 *    The answer bounds how long a human may spend reading the diff.
 * 3. `saved` — is `FILE_SAVED` honoured with contents that differ from
 *    `new_file_contents`? That is what would let the user edit the diff before
 *    accepting it. The reply carries `SAVED_MARKER`; grep the target file for
 *    it afterwards, because only the CLI can put it there.
 *
 * `SPIKE_OPENDIFF=<mode>[:<delay-ms>]`, modes `none` (default, `-32601`),
 * `accept`, `reject`, `saved`, `hang`.
 */
const [OPEN_DIFF_MODE, OPEN_DIFF_DELAY] = (
  process.env.SPIKE_OPENDIFF ?? "none"
).split(":");
const SAVED_MARKER = `SPIKE_EDIT_${randomBytes(3).toString("hex")}`;

// When the pending openDiff was received, so the close_tab that follows can be
// reported as an interval rather than a timestamp.
let openDiffAt = 0;

function openDiffResult(args?: Record<string, unknown>) {
  switch (OPEN_DIFF_MODE) {
    case "accept":
      return { content: [{ type: "text", text: "DIFF_ACCEPTED" }] };
    case "reject":
      return {
        content: [
          { type: "text", text: "DIFF_REJECTED" },
          { type: "text", text: String(args?.tab_name ?? "") },
        ],
      };
    // PROTOCOL.md's shape for "the user saved the diff buffer": the verdict and
    // the contents, as two blocks.
    case "saved":
      return {
        content: [
          { type: "text", text: "FILE_SAVED" },
          {
            type: "text",
            text: `${String(args?.new_file_contents ?? "")}\n${SAVED_MARKER}\n`,
          },
        ],
      };
    default:
      return null;
  }
}

/** True when this openDiff was handled here and must not fall through. */
function answerOpenDiff(
  id: string | number,
  args?: Record<string, unknown>,
): boolean {
  if (OPEN_DIFF_MODE === "none") return false;
  openDiffAt = Date.now();
  if (OPEN_DIFF_MODE === "hang") {
    console.error("⏱ openDiff held open — no answer will be sent");
    return true;
  }
  const result = openDiffResult(args);
  if (!result) return false;
  const delay = Number(OPEN_DIFF_DELAY ?? 0);
  const reply = () => {
    send({ jsonrpc: "2.0", id, result });
    console.error(`⏱ openDiff answered after ${Date.now() - openDiffAt}ms`);
  };
  if (delay > 0) setTimeout(reply, delay);
  else reply();
  return true;
}

// Unknown tools return null; the caller turns that into a JSON-RPC error and logs it.
function callTool(name: string | undefined, args?: Record<string, unknown>) {
  switch (name) {
    case "getCurrentSelection":
    case "getLatestSelection":
      return asText(selectionPayload());
    case "getOpenEditors":
      return asText({ tabs: [] });
    case "getWorkspaceFolders":
      return asText({
        success: true,
        folders: workspaces.map((w) => ({
          name: path.basename(w),
          uri: `file://${w}`,
          path: w,
        })),
        rootPath: workspaces[0],
      });
    case "getDiagnostics":
      return asText([]); // yazi has no LSP, so this is always empty
    // Honest stubs below: yazi is not an editor and cannot do these things, but
    // the CLI still needs to see that the tools exist.
    case "closeAllDiffTabs":
      return { content: [{ type: "text", text: "CLOSED_0_DIFF_TABS" }] };
    case "close_tab":
      return { content: [{ type: "text", text: "TAB_CLOSED" }] };
    case "openFile":
      return {
        content: [{ type: "text", text: `Opened file: ${args?.filePath}` }],
      };
    // No `openDiff` case. This spike returned `DIFF_REJECTED` on the reasoning
    // that yazi has no diff UI, so rejecting was the honest answer. Measured
    // 2026-08-08 against the real implementation: the CLI reads that as the user
    // refusing the change and cancels the edit outright, so anyone running this
    // spike would find their edits silently failing. Falling through to `-32601`
    // matches contract F5 — see docs/baseline.md.
    case "checkDocumentDirty":
    case "saveDocument":
      return asText({
        success: false,
        message: `Document not open: ${args?.filePath}`,
      });
    default:
      return null;
  }
}

fs.mkdirSync(IDE_DIR, { recursive: true, mode: 0o700 });
fs.mkdirSync(path.join(SPIKE_DIR, "fixtures"), { recursive: true });

let socket: Bun.ServerWebSocket<unknown> | null = null;

function send(obj: RpcMessage): void {
  log("out", obj);
  socket?.send(JSON.stringify(obj));
}

function pushSelection(): void {
  const p = selectionPayload();
  if (!socket || !p.success) return;
  send({
    jsonrpc: "2.0",
    method: "selection_changed",
    params: {
      text: p.text,
      filePath: p.filePath,
      fileUrl: `file://${p.filePath}`,
      selection: p.selection,
    },
  });
}

// What was mentioned last, so an unrelated edit to state.json does not resend
// the same set. An empty `mentions` clears it, which is how the same set can be
// pushed twice in one session.
let lastMentions = "";

/**
 * One `at_mentioned` per marked file, sent in the order state.json lists them.
 * A marked file in a file manager has no range, so the range is whatever
 * `mentionRange` says and omitted when it says nothing.
 */
function pushMentions(): void {
  const state = readState();
  const mentions = state.mentions ?? [];
  // The range is part of the key: re-sending the same files with a different
  // range is a distinct probe, not a repeat.
  const key = JSON.stringify([mentions, state.mentionRange ?? null]);
  if (!socket || key === lastMentions) return;
  lastMentions = key;
  const range = state.mentionRange;
  for (const filePath of mentions)
    send({
      jsonrpc: "2.0",
      method: "at_mentioned",
      params: range
        ? { filePath, lineStart: range[0], lineEnd: range[1] }
        : { filePath },
    });
}

function handle(msg: RpcMessage): void {
  log("in", msg);
  if (msg.id === undefined) return; // notification, no reply expected

  switch (msg.method) {
    case "initialize":
      send({
        jsonrpc: "2.0",
        id: msg.id,
        result: {
          protocolVersion: msg.params?.protocolVersion ?? "2025-03-26",
          capabilities: { tools: {} },
          serverInfo: { name: "yazi-claude-ide-spike", version: "0.0.0" },
        },
      });
      return;
    case "tools/list":
      send({ jsonrpc: "2.0", id: msg.id, result: { tools: ADVERTISED } });
      return;
    case "tools/call": {
      const name = msg.params?.name as string | undefined;
      const args = msg.params?.arguments as Record<string, unknown> | undefined;
      if (name === "openDiff" && answerOpenDiff(msg.id, args)) return;
      // The CLI sends close_tab twice per diff, with the openDiff tab_name.
      // Under `hang` that interval is the CLI's patience, measured.
      if (name === "close_tab" && openDiffAt)
        console.error(
          `⏱ close_tab ${Date.now() - openDiffAt}ms after openDiff (${args?.tab_name})`,
        );
      const result = callTool(name, args);
      if (result) send({ jsonrpc: "2.0", id: msg.id, result });
      else
        send({
          jsonrpc: "2.0",
          id: msg.id,
          error: {
            code: -32601,
            message: `Tool not found: ${msg.params?.name}`,
          },
        });
      return;
    }
    default:
      send({
        jsonrpc: "2.0",
        id: msg.id,
        error: { code: -32601, message: `Method not found: ${msg.method}` },
      });
  }
}

const server = Bun.serve({
  hostname: "127.0.0.1",
  port: 0,
  fetch(req, server) {
    const got = req.headers.get("x-claude-code-ide-authorization");
    if (got !== authToken) {
      console.error(
        `✗ auth failed, connection rejected (header=${got ? "present but wrong" : "missing"})`,
      );
      return new Response("Unauthorized", { status: 401 });
    }
    if (server.upgrade(req)) return undefined;
    return new Response("Upgrade Required", { status: 426 });
  },
  websocket: {
    open(ws) {
      socket = ws;
      console.error("✓ Claude connected");
      // Measured: Claude never calls getCurrentSelection on its own after
      // connecting, so push once. The delay lets the initialize handshake finish.
      setTimeout(pushSelection, 1500);
    },
    message(_ws, raw) {
      const text =
        typeof raw === "string" ? raw : new TextDecoder().decode(raw);
      try {
        handle(JSON.parse(text) as RpcMessage);
      } catch {
        console.error(`✗ non-JSON message: ${text.slice(0, 200)}`);
      }
    },
    close() {
      console.error("✗ connection closed");
      socket = null;
      lastMentions = ""; // the next connection is owed the current set again
    },
  },
});

const lockPath = path.join(IDE_DIR, `${server.port}.lock`);
fs.writeFileSync(
  lockPath,
  JSON.stringify({
    pid: process.pid,
    workspaceFolders: workspaces,
    // Not "yazi": a real sidecar may be running alongside this spike, and the
    // /ide picker lists lock files by ideName alone. Adoption is measured to
    // depend on workspaceFolders only (baseline.md), so renaming costs nothing.
    ideName: "yazi-spike",
    transport: "ws",
    authToken,
  }),
  { mode: 0o600 },
);
logStream = fs.createWriteStream(
  path.join(SPIKE_DIR, "fixtures", `session-${server.port}.jsonl`),
  { flags: "a" },
);

console.error(`listening 127.0.0.1:${server.port}`);
console.error(`lock      ${lockPath}`);
console.error(`workspace ${workspaces.join(", ")}`);
console.error(`state     ${STATE_FILE}`);
console.error(
  `tools     ${ADVERTISED.length} advertised / ${TOOLS.length} implemented`,
);
console.error(
  `openDiff  ${OPEN_DIFF_MODE}${OPEN_DIFF_DELAY ? ` after ${OPEN_DIFF_DELAY}ms` : ""}` +
    (OPEN_DIFF_MODE === "saved" ? ` · marker ${SAVED_MARKER}` : ""),
);

// Any edit to state.json pushes a selection_changed notification, and any
// change to `mentions` pushes one at_mentioned per marked file.
fs.watchFile(STATE_FILE, { interval: 300 }, () => {
  pushSelection();
  pushMentions();
});

function cleanup(): never {
  try {
    fs.unlinkSync(lockPath);
  } catch {}
  process.exit(0);
}
process.on("SIGINT", cleanup);
process.on("SIGTERM", cleanup);
