// The WebSocket server Claude Code talks MCP over. Contract clauses A5, D, E.
// The lock file is src/lock.ts's job; the sidecar entry point composes the two.
import { newAuthToken } from "./lock.ts";
import { ADVERTISED, callTool, selectionPayload } from "./tools.ts";
import type { ToolContext } from "./tools.ts";

export type Sidecar = {
  port: number;
  hostname: string;
  authToken: string;
  setFocus(filePath: string | null): void;
  focusedFile(): string | null;
  stop(): void;
};

type Rpc = {
  jsonrpc: "2.0";
  id?: string | number;
  method?: string;
  params?: Record<string, unknown>;
  result?: unknown;
  error?: { code: number; message: string };
};

// Measured against Claude Code 2.1.223 — the CLI sends this, PROTOCOL.md's
// 2025-03-26 is stale. Used only when a client omits its own version.
const PROTOCOL_VERSION = "2025-11-25";

export function startSidecar(opts: {
  workspaceFolders: () => string[];
  reveal: (filePath: string) => void;
  authToken?: string;
  port?: number;
}): Sidecar {
  const authToken = opts.authToken ?? newAuthToken();

  let focused: string | null = null;
  // What the live connection has already been told, so the same path never goes
  // out twice in a row (D6). Cleared on disconnect: the next connection is owed
  // one push of the then-current file (D3, D7).
  let lastPushed: string | null = null;
  const sockets = new Set<Bun.ServerWebSocket<unknown>>();

  const context: ToolContext = {
    focusedFile: () => focused,
    workspaceFolders: opts.workspaceFolders,
    reveal: opts.reveal,
  };

  function send(ws: Bun.ServerWebSocket<unknown>, msg: Rpc): void {
    ws.send(JSON.stringify(msg));
  }

  /**
   * D1-D2. Null when there is nothing to say: nothing focused, or a path that
   * stopped statting since `setFocus` accepted it. A half-built frame would
   * carry `filePath: undefined`, which D2 forbids and D5 rules out entirely.
   */
  function selectionFrame(): Rpc | null {
    if (!focused) return null;
    const payload = selectionPayload(focused);
    if (!payload.success) return null;
    return {
      jsonrpc: "2.0",
      method: "selection_changed",
      params: {
        text: payload.text,
        filePath: payload.filePath,
        fileUrl: `file://${payload.filePath}`,
        selection: payload.selection,
      },
    };
  }

  /** Broadcast a focus change. Silent with nothing new or nobody listening (D5, D6, D7). */
  function push(): void {
    if (focused === lastPushed || sockets.size === 0) return;
    const frame = selectionFrame();
    if (!frame) return;
    for (const ws of sockets) send(ws, frame);
    lastPushed = focused;
  }

  function handle(ws: Bun.ServerWebSocket<unknown>, msg: Rpc): void {
    if (msg.id === undefined) return; // a notification is never answered (E4)

    switch (msg.method) {
      case "initialize":
        send(ws, {
          jsonrpc: "2.0",
          id: msg.id,
          result: {
            protocolVersion: msg.params?.protocolVersion ?? PROTOCOL_VERSION,
            capabilities: { tools: {} },
            serverInfo: { name: "yazi", version: "0.1.0" },
          },
        });
        return;
      case "tools/list":
        send(ws, { jsonrpc: "2.0", id: msg.id, result: { tools: ADVERTISED } });
        return;
      case "tools/call": {
        const name = msg.params?.name as string;
        const result = callTool(
          name,
          (msg.params?.arguments as Record<string, unknown>) ?? {},
          context,
        );
        // callTool returns null for a tool we do not implement (F4).
        if (result) send(ws, { jsonrpc: "2.0", id: msg.id, result });
        else
          send(ws, {
            jsonrpc: "2.0",
            id: msg.id,
            error: { code: -32601, message: `Tool not found: ${name}` },
          });
        return;
      }
      default:
        send(ws, {
          jsonrpc: "2.0",
          id: msg.id,
          error: { code: -32601, message: `Method not found: ${msg.method}` },
        });
    }
  }

  const server = Bun.serve({
    hostname: "127.0.0.1", // loopback only, never a routable interface (A5)
    port: opts.port ?? 0,
    fetch(req, server) {
      if (req.headers.get("x-claude-code-ide-authorization") !== authToken)
        return new Response("Unauthorized", { status: 401 }); // E1
      if (server.upgrade(req)) return undefined;
      return new Response("Upgrade Required", { status: 426 });
    },
    websocket: {
      open(ws) {
        sockets.add(ws);
        // D3 is owed to each connection, not to the sidecar: a session joining
        // while another socket already holds the current path must still be
        // told the file. Going through push() would let `lastPushed` — which
        // tracks the broadcast stream for D6 — swallow this one.
        const frame = selectionFrame();
        if (!frame) return;
        send(ws, frame);
        lastPushed = focused;
      },
      message(ws, raw) {
        const text =
          typeof raw === "string" ? raw : new TextDecoder().decode(raw);
        let msg: unknown;
        try {
          msg = JSON.parse(text);
        } catch {
          // No id can be recovered, so there is nobody to reply to (E3, E5).
          console.error(`non-JSON frame dropped: ${text.slice(0, 200)}`);
          return;
        }
        // `null` and the JSON scalars parse without throwing yet carry no id,
        // so they are just as unanswerable — and reading `.id` off `null` would
        // take the sidecar down with a TypeError (E3, E5).
        if (typeof msg !== "object" || msg === null) {
          console.error(`non-object frame dropped: ${text.slice(0, 200)}`);
          return;
        }
        handle(ws, msg as Rpc);
      },
      close(ws) {
        sockets.delete(ws);
        if (sockets.size === 0) lastPushed = null; // D7
      },
    },
  });

  return {
    port: server.port,
    hostname: server.hostname,
    authToken,
    setFocus(filePath) {
      // selectionPayload owns the "is this a file?" decision, so focus and the
      // payload can never disagree, and a stat that throws — ENOTDIR, ELOOP,
      // EACCES — clears focus instead of escaping into the DDS stream (C5, D5).
      focused = selectionPayload(filePath).success ? filePath : null;
      push();
    },
    focusedFile: () => focused,
    stop() {
      server.stop(true);
    },
  };
}
