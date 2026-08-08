// Contract clauses A5, D1-D7, and E1-E5.
import { afterEach, describe, expect, test } from "bun:test";
import fs from "node:fs";
import path from "node:path";

import { startSidecar } from "../src/server.ts";
import type { Sidecar } from "../src/server.ts";

const REPO = path.resolve(import.meta.dir, "..");
const FILE = path.join(REPO, "package.json");
const OTHER = path.join(REPO, "README.md");

let running: Sidecar | null = null;

afterEach(() => {
  running?.stop();
  running = null;
});

function start(): Sidecar {
  running = startSidecar({
    workspaceFolders: () => [REPO],
    reveal: () => {},
  });
  return running;
}

/** A client socket that queues every frame, so a test can await the next one. */
class Client {
  #queue: Record<string, unknown>[] = [];
  #waiters: ((msg: Record<string, unknown>) => void)[] = [];
  #socket: WebSocket;

  constructor(socket: WebSocket) {
    this.#socket = socket;
    socket.addEventListener("message", (event) => {
      const msg = JSON.parse(String(event.data));
      const waiter = this.#waiters.shift();
      if (waiter) waiter(msg);
      else this.#queue.push(msg);
    });
  }

  static connect(sidecar: Sidecar, token = sidecar.authToken): Promise<Client> {
    const socket = new WebSocket(`ws://127.0.0.1:${sidecar.port}`, {
      headers: { "x-claude-code-ide-authorization": token },
    });
    return new Promise((resolve, reject) => {
      socket.addEventListener("open", () => resolve(new Client(socket)));
      socket.addEventListener("error", reject);
    });
  }

  send(msg: Record<string, unknown>): void {
    this.#socket.send(JSON.stringify(msg));
  }

  raw(text: string): void {
    this.#socket.send(text);
  }

  next(timeoutMs = 2000): Promise<Record<string, unknown>> {
    const queued = this.#queue.shift();
    if (queued) return Promise.resolve(queued);
    return new Promise((resolve, reject) => {
      const waiter = (msg: Record<string, unknown>) => {
        clearTimeout(timer);
        resolve(msg);
      };
      // A timed-out waiter has to leave the queue. `silence()` times out by
      // design, and a waiter left behind swallows the next frame — which is
      // the frame the following assertion is waiting for.
      const timer = setTimeout(() => {
        this.#waiters.splice(this.#waiters.indexOf(waiter), 1);
        reject(new Error("no frame arrived"));
      }, timeoutMs);
      this.#waiters.push(waiter);
    });
  }

  /** Resolves to null when nothing arrives — the assertion for "must not push". */
  async silence(ms = 400): Promise<Record<string, unknown> | null> {
    try {
      return await this.next(ms);
    } catch {
      return null;
    }
  }

  async call(id: number, method: string, params?: Record<string, unknown>) {
    this.send({ jsonrpc: "2.0", id, method, params });
    return await this.next();
  }

  close(): void {
    this.#socket.close();
  }
}

describe("A5/E1. binding and authentication", () => {
  test("A5 the server binds loopback only", () => {
    expect(start().hostname).toBe("127.0.0.1");
  });

  test("E1 a missing token is refused with 401", async () => {
    const res = await fetch(`http://127.0.0.1:${start().port}`);
    expect(res.status).toBe(401);
  });

  test("E1 a wrong token is refused and never upgraded", async () => {
    const sidecar = start();
    const res = await fetch(`http://127.0.0.1:${sidecar.port}`, {
      headers: { "x-claude-code-ide-authorization": "0".repeat(32) },
    });
    expect(res.status).toBe(401);
    await expect(Client.connect(sidecar, "0".repeat(32))).rejects.toBeDefined();
  });

  test("E1 the correct token connects", async () => {
    const client = await Client.connect(start());
    expect(client).toBeInstanceOf(Client);
    client.close();
  });
});

describe("E. error semantics", () => {
  test("E2 an unknown method returns -32601", async () => {
    const client = await Client.connect(start());
    const reply = await client.call(7, "resources/list");
    expect(reply.error).toMatchObject({ code: -32601 });
  });

  test("E2 an unimplemented tool returns -32601", async () => {
    const client = await Client.connect(start());
    const reply = await client.call(8, "tools/call", { name: "executeCode" });
    expect(reply.error).toMatchObject({ code: -32601 });
  });

  test("E3/E5 a non-JSON frame is dropped and the sidecar keeps serving", async () => {
    const client = await Client.connect(start());
    client.raw("<not json>");
    expect(await client.silence()).toBeNull();
    expect(await client.call(9, "tools/list")).toHaveProperty("result");
  });

  test("E3/E5 a JSON frame that is not an object is dropped, and the sidecar lives", async () => {
    const client = await Client.connect(start());
    // `null` parses, so the E3 catch never sees it; reading `.id` off it used
    // to take the whole sidecar down with a TypeError.
    for (const frame of ["null", "5", '"x"', "false", "[]"]) client.raw(frame);
    expect(await client.silence()).toBeNull();
    expect(await client.call(10, "tools/list")).toHaveProperty("result");
  });

  test("E4 a notification is never answered", async () => {
    const client = await Client.connect(start());
    client.send({ jsonrpc: "2.0", method: "notifications/initialized" });
    expect(await client.silence()).toBeNull();
  });
});

describe("MCP handshake", () => {
  test("initialize echoes the client's protocol version and names the server", async () => {
    const client = await Client.connect(start());
    const reply = await client.call(1, "initialize", {
      protocolVersion: "2025-11-25",
      capabilities: {},
    });
    expect(reply.result).toMatchObject({
      protocolVersion: "2025-11-25",
      capabilities: { tools: {} },
    });
    expect(
      (reply.result as { serverInfo: { name: string } }).serverInfo.name,
    ).toBe("yazi");
  });

  test("tools/list advertises the four MVP tools", async () => {
    const client = await Client.connect(start());
    const reply = await client.call(2, "tools/list");
    const names = (reply.result as { tools: { name: string }[] }).tools.map(
      (t) => t.name,
    );
    expect(names).toEqual([
      "getCurrentSelection",
      "getLatestSelection",
      "getWorkspaceFolders",
      "getOpenEditors",
    ]);
  });
});

describe("D. the selection_changed push", () => {
  test("D3 connecting pushes the current file once", async () => {
    const sidecar = start();
    sidecar.setFocus(FILE);
    const client = await Client.connect(sidecar);

    const push = await client.next();
    expect(push).toMatchObject({ jsonrpc: "2.0", method: "selection_changed" });
    expect(await client.silence()).toBeNull();
  });

  test("D3 a second connection is pushed too, while the first is still open", async () => {
    const sidecar = start();
    sidecar.setFocus(FILE);
    const first = await Client.connect(sidecar);
    await first.next();

    // Two Claude sessions against one yazi. The second is owed its own push;
    // one sidecar-wide "already told them" flag used to swallow it.
    const second = await Client.connect(sidecar);
    expect((await second.next()).params).toMatchObject({ filePath: FILE });
    expect(await second.silence()).toBeNull();
    expect(await first.silence()).toBeNull(); // and the first is not told twice (D6)
  });

  test("D4 a focus change after a second connection reaches both", async () => {
    const sidecar = start();
    sidecar.setFocus(FILE);
    const first = await Client.connect(sidecar);
    await first.next();
    const second = await Client.connect(sidecar);
    await second.next();

    sidecar.setFocus(OTHER);
    expect((await first.next()).params).toMatchObject({ filePath: OTHER });
    expect((await second.next()).params).toMatchObject({ filePath: OTHER });
  });

  test("D1 the push is a notification, with no id", async () => {
    const sidecar = start();
    sidecar.setFocus(FILE);
    const client = await Client.connect(sidecar);
    expect(await client.next()).not.toHaveProperty("id");
  });

  test("D2 the push carries path, url, empty text, and an empty selection", async () => {
    const sidecar = start();
    sidecar.setFocus(FILE);
    const client = await Client.connect(sidecar);

    expect((await client.next()).params).toEqual({
      text: "",
      filePath: FILE,
      fileUrl: `file://${FILE}`,
      selection: {
        start: { line: 0, character: 0 },
        end: { line: 0, character: 0 },
        isEmpty: true,
      },
    });
  });

  test("D3 connecting with nothing focused pushes nothing", async () => {
    const client = await Client.connect(start());
    expect(await client.silence()).toBeNull();
  });

  test("D4 a changed focus pushes the new file", async () => {
    const sidecar = start();
    sidecar.setFocus(FILE);
    const client = await Client.connect(sidecar);
    await client.next(); // the connect-time push

    sidecar.setFocus(OTHER);
    expect((await client.next()).params).toMatchObject({ filePath: OTHER });
  });

  test("D5 focusing a directory pushes nothing and clears the selection", async () => {
    const sidecar = start();
    sidecar.setFocus(FILE);
    const client = await Client.connect(sidecar);
    await client.next();

    sidecar.setFocus(path.join(REPO, "spike"));

    expect(await client.silence()).toBeNull();
    const reply = await client.call(3, "tools/call", {
      name: "getCurrentSelection",
    });
    const payload = JSON.parse(
      (reply.result as { content: [{ text: string }] }).content[0].text,
    );
    expect(payload).toEqual({
      success: false,
      message: "No active editor found",
    });
  });

  test("D5 focusing a path that does not stat pushes nothing", async () => {
    const sidecar = start();
    sidecar.setFocus(FILE);
    const client = await Client.connect(sidecar);
    await client.next();

    sidecar.setFocus(path.join(REPO, "gone-", "missing.txt"));
    expect(await client.silence()).toBeNull();
  });

  test("D5 a file that vanishes after focus is never pushed, not even half-built", async () => {
    const dir = fs.mkdtempSync(path.join(fs.realpathSync("/tmp"), "yci-gone-"));
    const file = path.join(dir, "vanishing.txt");
    fs.writeFileSync(file, "x");

    const sidecar = start();
    sidecar.setFocus(file); // accepted: it stats now
    fs.rmSync(file); // and is gone by the time anyone connects

    const client = await Client.connect(sidecar);
    expect(await client.silence()).toBeNull();
  });

  test("D5 focusing a path whose stat throws clears focus and stays silent", async () => {
    const dir = fs.mkdtempSync(path.join(fs.realpathSync("/tmp"), "yci-stat-"));
    const regular = path.join(dir, "regular.txt");
    fs.writeFileSync(regular, "x");
    const loop = path.join(dir, "a");
    fs.symlinkSync(path.join(dir, "b"), loop);
    fs.symlinkSync(loop, path.join(dir, "b"));

    const sidecar = start();
    sidecar.setFocus(FILE);
    const client = await Client.connect(sidecar);
    await client.next();

    // ENOTDIR and ELOOP: stats yazi can legitimately ask for, and that
    // `throwIfNoEntry: false` does not suppress.
    for (const bad of [path.join(regular, "child.txt"), loop]) {
      sidecar.setFocus(bad);
      expect(sidecar.focusedFile()).toBeNull();
      expect(await client.silence()).toBeNull();
    }
  });

  test("D6 the same path twice in a row pushes once", async () => {
    const sidecar = start();
    sidecar.setFocus(FILE);
    const client = await Client.connect(sidecar);
    await client.next();

    sidecar.setFocus(FILE);
    expect(await client.silence()).toBeNull();
  });

  test("D7 changes made while disconnected are not replayed", async () => {
    const sidecar = start();
    const first = await Client.connect(sidecar);
    sidecar.setFocus(FILE);
    await first.next();
    first.close();
    await Bun.sleep(100);

    sidecar.setFocus(OTHER);
    sidecar.setFocus(FILE);

    const second = await Client.connect(sidecar);
    expect((await second.next()).params).toMatchObject({ filePath: FILE });
    expect(await second.silence()).toBeNull();
  });

  test("D5 a symlink to a regular file is pushed as given", async () => {
    const dir = fs.mkdtempSync(path.join(fs.realpathSync("/tmp"), "yci-sym-"));
    const link = path.join(dir, "link.json");
    fs.symlinkSync(FILE, link);

    const sidecar = start();
    sidecar.setFocus(link);
    const client = await Client.connect(sidecar);
    expect((await client.next()).params).toMatchObject({ filePath: link });
  });
});

describe("H. the at_mentioned push", () => {
  const DIR = path.join(REPO, "src");
  const GONE = path.join(REPO, "no-such-file.txt");

  test("H4 each marked file gets its own notification, params exactly filePath", async () => {
    const sidecar = start();
    const client = await Client.connect(sidecar);

    sidecar.mention([FILE, OTHER]);
    const first = await client.next();
    expect(first.method).toBe("at_mentioned");
    expect(first.id).toBeUndefined(); // a notification is never answered
    // No lineStart/lineEnd: measured, `0` renders as `@file#L1` (baseline.md).
    expect(first.params).toEqual({ filePath: FILE });
    expect((await client.next()).params).toEqual({ filePath: OTHER });
  });

  test("H5 the notifications keep yazi's order", async () => {
    const sidecar = start();
    const client = await Client.connect(sidecar);

    sidecar.mention([OTHER, FILE]);
    expect((await client.next()).params).toEqual({ filePath: OTHER });
    expect((await client.next()).params).toEqual({ filePath: FILE });
  });

  test("H6 a directory is mentioned like any other path", async () => {
    // Measured: the CLI runs `ls` on a mentioned directory and `Read` on a
    // mentioned file. C5's file-only test guards selection_changed, not this.
    const sidecar = start();
    const client = await Client.connect(sidecar);

    sidecar.mention([DIR, FILE]);
    expect((await client.next()).params).toEqual({ filePath: DIR });
    expect((await client.next()).params).toEqual({ filePath: FILE });
  });

  test("H6 a set of only directories still sends", async () => {
    const sidecar = start();
    const client = await Client.connect(sidecar);

    sidecar.mention([DIR]);
    expect((await client.next()).params).toEqual({ filePath: DIR });
  });

  test("H6 a path that no longer stats is skipped without losing the rest", async () => {
    const sidecar = start();
    const client = await Client.connect(sidecar);

    sidecar.mention([GONE, FILE]);
    expect((await client.next()).params).toEqual({ filePath: FILE });
    expect(await client.silence()).toBeNull();
  });

  test("H7 an empty set falls back to the path under the cursor", async () => {
    const sidecar = start();
    sidecar.setFocus(FILE);
    const client = await Client.connect(sidecar);
    await client.next(); // the D3 selection_changed owed to this connection

    sidecar.mention([]);
    expect((await client.next()).params).toEqual({ filePath: FILE });
  });

  test("H7 the fallback fires while the cursor sits on a directory", async () => {
    // The case the focused file cannot serve: C5 leaves `focused` null here, so
    // a fallback reading it would go silent exactly when the user stands on a
    // folder — which is now a thing worth mentioning (H6).
    const sidecar = start();
    sidecar.setFocus(DIR);
    expect(sidecar.focusedFile()).toBeNull();
    const client = await Client.connect(sidecar);

    sidecar.mention([]);
    expect((await client.next()).params).toEqual({ filePath: DIR });
  });

  test("H7 the cursor path is not pushed as a selection", async () => {
    // Tracking the hovered directory must not leak into D5's stream.
    const sidecar = start();
    sidecar.setFocus(DIR);
    const client = await Client.connect(sidecar);
    expect(await client.silence()).toBeNull();
  });

  test("H7 an empty set with nothing hovered sends nothing", async () => {
    const sidecar = start();
    const client = await Client.connect(sidecar);

    sidecar.mention([]);
    expect(await client.silence()).toBeNull();
  });

  test("H7 a cursor path that has since vanished sends nothing", async () => {
    const sidecar = start();
    sidecar.setFocus(GONE);
    const client = await Client.connect(sidecar);

    sidecar.mention([]);
    expect(await client.silence()).toBeNull();
  });

  test("H8 a gesture with no connection open is not queued for replay", async () => {
    const sidecar = start();
    sidecar.mention([FILE, OTHER]);

    const client = await Client.connect(sidecar);
    expect(await client.silence()).toBeNull();
  });

  test("H9 every open connection receives the set", async () => {
    const sidecar = start();
    const first = await Client.connect(sidecar);
    const second = await Client.connect(sidecar);

    sidecar.mention([FILE]);
    expect((await first.next()).params).toEqual({ filePath: FILE });
    expect((await second.next()).params).toEqual({ filePath: FILE });
  });

  test("H9 the same set twice sends twice, unlike D6", async () => {
    const sidecar = start();
    const client = await Client.connect(sidecar);

    sidecar.mention([FILE]);
    sidecar.mention([FILE]);
    expect((await client.next()).params).toEqual({ filePath: FILE });
    expect((await client.next()).params).toEqual({ filePath: FILE });
  });

  test("H4 mentioning does not disturb the selection_changed stream", async () => {
    const sidecar = start();
    sidecar.setFocus(FILE);
    const client = await Client.connect(sidecar);
    expect((await client.next()).method).toBe("selection_changed");

    sidecar.mention([OTHER]);
    expect((await client.next()).method).toBe("at_mentioned");

    // D6 still measures against what selection_changed last pushed, so the
    // mention must not have counted as one.
    sidecar.setFocus(OTHER);
    const next = await client.next();
    expect(next.method).toBe("selection_changed");
    expect(next.params).toMatchObject({ filePath: OTHER });
  });
});
