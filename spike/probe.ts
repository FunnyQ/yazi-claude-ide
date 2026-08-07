#!/usr/bin/env bun
// Pretends to be Claude, to verify fake-ide's auth + MCP handshake + tools/call.
// Usage: bun spike/probe.ts            → reads the most recent lock file
//        bun spike/probe.ts <port>
import fs from "node:fs";
import { homedir } from "node:os";
import path from "node:path";

const IDE_DIR = path.join(
  process.env.CLAUDE_CONFIG_DIR || path.join(homedir(), ".claude"),
  "ide",
);

const port =
  process.argv[2] ??
  fs
    .readdirSync(IDE_DIR)
    .filter((f) => f.endsWith(".lock"))
    .map((f) => ({ f, m: fs.statSync(path.join(IDE_DIR, f)).mtimeMs }))
    .sort((a, b) => b.m - a.m)[0]
    ?.f.replace(".lock", "");

if (!port) {
  console.error("no lock file found");
  process.exit(1);
}

const lock = JSON.parse(
  fs.readFileSync(path.join(IDE_DIR, `${port}.lock`), "utf8"),
);
console.log(
  `lock: port=${port} ideName=${lock.ideName} workspace=${lock.workspaceFolders[0]}`,
);

let pass = 0;
let fail = 0;
const check = (name: string, ok: boolean, detail = "") => {
  console.log(`${ok ? "✓" : "✗"} ${name}${detail ? ` — ${detail}` : ""}`);
  ok ? pass++ : fail++;
};

check(
  "authToken is 32 lowercase hex chars",
  /^[0-9a-f]{32}$/.test(lock.authToken),
);
check("transport is ws", lock.transport === "ws");

// A wrong token must be rejected.
await new Promise<void>((resolve) => {
  const bad = new WebSocket(`ws://127.0.0.1:${port}`, {
    headers: { "x-claude-code-ide-authorization": "0".repeat(32) },
  } as never);
  bad.onopen = () => {
    check("wrong token is rejected", false, "it connected anyway");
    bad.close();
    resolve();
  };
  bad.onerror = () => {
    check("wrong token is rejected", true);
    resolve();
  };
});

const ws = new WebSocket(`ws://127.0.0.1:${port}`, {
  headers: { "x-claude-code-ide-authorization": lock.authToken },
} as never);

let nextId = 1;
const pending = new Map<number, (v: unknown) => void>();
const call = (method: string, params?: unknown) =>
  new Promise<Record<string, unknown>>((resolve) => {
    const id = nextId++;
    pending.set(id, resolve as (v: unknown) => void);
    ws.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
  });

ws.onmessage = (e) => {
  const msg = JSON.parse(String(e.data));
  if (msg.id !== undefined) pending.get(msg.id)?.(msg);
};

await new Promise<void>((resolve, reject) => {
  ws.onopen = () => resolve();
  ws.onerror = () => reject(new Error("correct token could not connect"));
});
check("correct token connects", true);

const init = await call("initialize", {
  protocolVersion: "2025-03-26",
  capabilities: {},
  clientInfo: { name: "probe", version: "0" },
});
check(
  "initialize returns serverInfo",
  Boolean((init.result as Record<string, unknown>)?.serverInfo),
);

const list = await call("tools/list");
const tools = (
  (list.result as { tools?: { name: string }[] })?.tools ?? []
).map((t) => t.name);
// The advertised count varies with SPIKE_MINIMAL_TOOLS, so assert on the four
// read-only tools the MVP actually depends on rather than on a total.
const CORE = [
  "getCurrentSelection",
  "getLatestSelection",
  "getOpenEditors",
  "getWorkspaceFolders",
];
check(
  "tools/list advertises the core read-only tools",
  CORE.every((t) => tools.includes(t)),
  `${tools.length} advertised: ${tools.join(", ")}`,
);

const sel = await call("tools/call", { name: "getCurrentSelection" });
const selText =
  (sel.result as { content?: { text: string }[] })?.content?.[0]?.text ?? "";
const selData = JSON.parse(selText);
check(
  "getCurrentSelection returns the filePath from state",
  Boolean(selData.filePath),
  selData.filePath,
);
check(
  "getCurrentSelection returns an empty text (the spike's independent variable)",
  selData.text === "",
);

const wf = await call("tools/call", { name: "getWorkspaceFolders" });
const wfData = JSON.parse(
  (wf.result as { content?: { text: string }[] })?.content?.[0]?.text ?? "{}",
);
check(
  "getWorkspaceFolders returns the workspace",
  wfData.rootPath === lock.workspaceFolders[0],
  wfData.rootPath,
);

const unknown = await call("tools/call", {
  name: "executeCode",
  arguments: { code: "print(1)" },
});
check(
  "an unimplemented tool returns a JSON-RPC error",
  (unknown.error as { code: number })?.code === -32601,
);

ws.close();
console.log(`\n${pass} passed, ${fail} failed`);
process.exit(fail === 0 ? 0 : 1);
