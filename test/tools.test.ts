// Contract clauses C1-C6, E2, and F1-F4.
import { describe, expect, test } from "bun:test";
import fs from "node:fs";
import path from "node:path";

import { ADVERTISED, callTool, selectionPayload } from "../src/tools.ts";
import type { ToolContext } from "../src/tools.ts";

const REPO = path.resolve(import.meta.dir, "..");
const FILE = path.join(REPO, "package.json");

function ctx(focused: string | null, revealed: string[] = []): ToolContext {
  return {
    focusedFile: () => focused,
    workspaceFolders: () => [REPO, path.join(REPO, "spike")],
    reveal: (p) => void revealed.push(p),
  };
}

/** MCP wraps every result in a text block. A missing one is the test failing. */
function textOf(result: ReturnType<typeof callTool>): string {
  const text = result?.content[0]?.text;
  if (text === undefined) throw new Error("tool returned no text block");
  return text;
}

/** The payload is JSON inside that text block. */
function payloadOf(result: ReturnType<typeof callTool>) {
  return JSON.parse(textOf(result));
}

describe("C. selection payloads", () => {
  test("C1 getCurrentSelection and getLatestSelection return the same shape", () => {
    const c = ctx(FILE);
    expect(payloadOf(callTool("getCurrentSelection", {}, c))).toEqual(
      payloadOf(callTool("getLatestSelection", {}, c)),
    );
  });

  test("C2 a focused file yields success with an empty zero-width selection", () => {
    expect(selectionPayload(FILE)).toEqual({
      success: true,
      filePath: FILE,
      text: "",
      selection: {
        start: { line: 0, character: 0 },
        end: { line: 0, character: 0 },
        isEmpty: true,
      },
    });
  });

  test("C3 filePath is absolute and unresolved", () => {
    const payload = payloadOf(callTool("getCurrentSelection", {}, ctx(FILE)));
    expect(payload.filePath).toBe(FILE);
    expect(path.isAbsolute(payload.filePath)).toBe(true);
  });

  test("C4 text is empty even for a file with contents", () => {
    const payload = payloadOf(callTool("getCurrentSelection", {}, ctx(FILE)));
    expect(payload.text).toBe("");
  });

  test("C5 no focused file yields success:false, not an error", () => {
    const result = callTool("getCurrentSelection", {}, ctx(null));
    expect(result).not.toBeNull();
    expect(payloadOf(result)).toEqual({
      success: false,
      message: "No active editor found",
    });
  });

  test("C5 a file that vanished after focus yields success:false", () => {
    const dir = fs.mkdtempSync(path.join(fs.realpathSync("/tmp"), "yci-gone-"));
    const file = path.join(dir, "vanishing.txt");
    fs.writeFileSync(file, "x");
    expect(selectionPayload(file).success).toBe(true);

    fs.rmSync(file);
    expect(selectionPayload(file)).toEqual({
      success: false,
      message: "No active editor found",
    });
  });

  // `throwIfNoEntry: false` covers ENOENT alone, so these three are the cases a
  // suppressed stat would have thrown on instead of answering (C5, E5).
  test("C5 a path whose stat throws yields success:false, never a throw", () => {
    const dir = fs.mkdtempSync(path.join(fs.realpathSync("/tmp"), "yci-stat-"));

    const notDir = path.join(dir, "regular.txt");
    fs.writeFileSync(notDir, "x");
    const under = path.join(notDir, "child.txt"); // ENOTDIR

    const loop = path.join(dir, "a");
    fs.symlinkSync(path.join(dir, "b"), loop);
    fs.symlinkSync(loop, path.join(dir, "b")); // ELOOP

    for (const bad of [under, loop]) {
      expect(selectionPayload(bad)).toEqual({
        success: false,
        message: "No active editor found",
      });
    }
  });

  test("C6 getOpenEditors returns no tabs", () => {
    expect(payloadOf(callTool("getOpenEditors", {}, ctx(FILE)))).toEqual({
      tabs: [],
    });
  });

  test("B6 getWorkspaceFolders mirrors the lock file, anchor as rootPath", () => {
    expect(payloadOf(callTool("getWorkspaceFolders", {}, ctx(FILE)))).toEqual({
      success: true,
      folders: [
        { name: path.basename(REPO), uri: `file://${REPO}`, path: REPO },
        {
          name: "spike",
          uri: `file://${path.join(REPO, "spike")}`,
          path: path.join(REPO, "spike"),
        },
      ],
      rootPath: REPO,
    });
  });
});

describe("F. tools that are out of scope but still called", () => {
  test("F1 exactly four tools are advertised", () => {
    expect(ADVERTISED.map((t) => t.name)).toEqual([
      "getCurrentSelection",
      "getLatestSelection",
      "getWorkspaceFolders",
      "getOpenEditors",
    ]);
  });

  test("F1 every advertised tool declares an object input schema", () => {
    for (const tool of ADVERTISED) {
      expect(tool.description.length).toBeGreaterThan(0);
      expect(tool.inputSchema.type).toBe("object");
    }
  });

  test("F2 unadvertised-but-called tools answer instead of erroring", () => {
    const c = ctx(FILE);
    const answer = (name: string, args = {}) => textOf(callTool(name, args, c));

    expect(answer("closeAllDiffTabs")).toBe("CLOSED_0_DIFF_TABS");
    expect(answer("close_tab", { tab_name: "x" })).toBe("TAB_CLOSED");
    expect(JSON.parse(answer("getDiagnostics"))).toEqual([]);
    for (const name of ["checkDocumentDirty", "saveDocument"]) {
      expect(JSON.parse(answer(name, { filePath: FILE }))).toEqual({
        success: false,
        message: `Document not open: ${FILE}`,
      });
    }
  });

  test("F3 openFile reveals the file in yazi and says so", () => {
    const revealed: string[] = [];
    const result = callTool(
      "openFile",
      { filePath: FILE },
      ctx(FILE, revealed),
    );
    expect(revealed).toEqual([FILE]);
    expect(textOf(result)).toBe(`Opened file: ${FILE}`);
  });

  test("F3 openFile without a filePath reveals nothing and reports failure", () => {
    const revealed: string[] = [];
    const result = callTool("openFile", {}, ctx(FILE, revealed));
    expect(revealed).toEqual([]);
    expect(payloadOf(result)).toEqual({
      success: false,
      message: "openFile requires filePath",
    });
  });

  test("F4/E2 an unknown tool is refused, so the caller can raise -32601", () => {
    expect(callTool("executeCode", { code: "print(1)" }, ctx(FILE))).toBeNull();
  });

  test("F5 openDiff is refused, because answering it cancels the edit", () => {
    // Not an omission: a benign-looking `DIFF_REJECTED` reads to the CLI as the
    // user rejecting the change, and the edit never lands. Measured, see
    // docs/baseline.md. -32601 is the only answer that leaves editing working.
    expect(
      callTool(
        "openDiff",
        {
          old_file_path: FILE,
          new_file_path: FILE,
          new_file_contents: "x",
          tab_name: "t",
        },
        ctx(FILE),
      ),
    ).toBeNull();
  });
});
