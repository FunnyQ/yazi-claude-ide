#!/usr/bin/env bun
// Pretends to be Claude Code for exactly one `openDiff`, so section J can be
// driven against a real sidecar without an interactive Claude session.
// Usage: bun spike/diff-client.ts <ide-lock-dir> <file-to-diff>
import fs from "node:fs";
import path from "node:path";

const [ideDir, oldPath] = process.argv.slice(2);
const port = fs.readdirSync(ideDir).filter((f) => f.endsWith(".lock"))[0]!.replace(".lock", "");
const lock = JSON.parse(fs.readFileSync(path.join(ideDir, `${port}.lock`), "utf8"));

const ws = new WebSocket(`ws://127.0.0.1:${port}`, {
  headers: { "x-claude-code-ide-authorization": lock.authToken },
} as any);
const started = Date.now();

ws.onopen = () =>
  ws.send(JSON.stringify({
    jsonrpc: "2.0", id: 1, method: "tools/call",
    params: { name: "openDiff", arguments: {
      old_file_path: oldPath,
      new_file_path: oldPath,
      new_file_contents: "one\nTWO\nthree\n",
      tab_name: "✻ [Claude Code] one.txt (5c8bea) ⧉",
    }},
  }));

ws.onmessage = (e) => {
  const msg = JSON.parse(String(e.data));
  if (msg.id !== 1) return;                       // ignore the D3 selection push
  console.log(`REPLY +${Date.now() - started}ms ${JSON.stringify(msg)}`);
  process.exit(0);
};
setTimeout(() => { console.log("NO REPLY in 120s"); process.exit(1); }, 120_000);
