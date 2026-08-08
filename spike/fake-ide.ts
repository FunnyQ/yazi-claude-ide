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

function log(dir: "in" | "out", msg: unknown): void {
  const line = JSON.stringify({ dir, msg })
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
      const result = callTool(
        msg.params?.name as string | undefined,
        msg.params?.arguments as Record<string, unknown> | undefined,
      );
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
    },
  },
});

const lockPath = path.join(IDE_DIR, `${server.port}.lock`);
fs.writeFileSync(
  lockPath,
  JSON.stringify({
    pid: process.pid,
    workspaceFolders: workspaces,
    ideName: "yazi",
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

// Any edit to state.json pushes a selection_changed notification.
fs.watchFile(STATE_FILE, { interval: 300 }, pushSelection);

function cleanup(): never {
  try {
    fs.unlinkSync(lockPath);
  } catch {}
  process.exit(0);
}
process.on("SIGINT", cleanup);
process.on("SIGTERM", cleanup);
