// The MCP tool surface. Contract clauses B6, C, E2, and F.
import fs from "node:fs";
import path from "node:path";

/** Everything a tool needs from the sidecar, so this file stays free of state (C, B6, F3). */
export type ToolContext = {
  focusedFile: () => string | null;
  workspaceFolders: () => string[];
  reveal: (filePath: string) => void;
};

export type Selection = {
  start: { line: number; character: number };
  end: { line: number; character: number };
  isEmpty: boolean;
};

export type SelectionPayload =
  | { success: false; message: string }
  | { success: true; filePath: string; text: string; selection: Selection };

/** Every tool result is a JSON string inside a single text block — see baseline.md. */
export type ToolResult = { content: { type: "text"; text: string }[] };

/** The four tools of F1, in the order `tools/list` advertises them. */
export const ADVERTISED = [
  {
    name: "getCurrentSelection",
    description: "Get the file yazi's cursor is on",
    inputSchema: { type: "object", properties: {} },
  },
  {
    name: "getLatestSelection",
    description: "Get the most recent file yazi's cursor was on",
    inputSchema: { type: "object", properties: {} },
  },
  {
    name: "getWorkspaceFolders",
    description: "Get the workspace folders yazi is browsing",
    inputSchema: { type: "object", properties: {} },
  },
  {
    name: "getOpenEditors",
    description: "Get the open editor tabs; yazi has none",
    inputSchema: { type: "object", properties: {} },
  },
] as const;

const EMPTY_SELECTION: Selection = {
  start: { line: 0, character: 0 },
  end: { line: 0, character: 0 },
  isEmpty: true,
};

const asText = (obj: unknown): ToolResult => ({
  content: [{ type: "text", text: JSON.stringify(obj) }],
});

const asPlain = (text: string): ToolResult => ({
  content: [{ type: "text", text }],
});

/**
 * stat, not lstat: a symlink to a regular file is a file (B5). The throw is
 * caught rather than suppressed with `throwIfNoEntry` — that flag covers ENOENT
 * only, while a path under a file (ENOTDIR), a symlink loop (ELOOP), and an
 * unreadable parent (EACCES) all still throw. C5 calls every one of them "no
 * active editor", and E5 forbids any of them killing the sidecar.
 */
function isFile(filePath: string): boolean {
  try {
    return fs.statSync(filePath).isFile();
  } catch {
    return false;
  }
}

/**
 * C2-C5. The stat is what catches "a directory focused, or a path that no longer
 * stats" (C5) — focus is validated when it is set, but the file can vanish after.
 * It follows symlinks deliberately: the link target decides whether this is a
 * regular file, while `filePath` stays the unresolved path yazi reported (B5, C3).
 */
export function selectionPayload(filePath: string | null): SelectionPayload {
  if (filePath && isFile(filePath)) {
    // `text` is empty by contract: the sidecar never reads user files (C4).
    return { success: true, filePath, text: "", selection: EMPTY_SELECTION };
  }
  return { success: false, message: "No active editor found" };
}

/**
 * Runs one `tools/call`. Returns null for a tool this sidecar does not implement,
 * which the caller turns into JSON-RPC -32601 (E2, F4).
 */
export function callTool(
  name: string,
  args: Record<string, unknown>,
  ctx: ToolContext,
): ToolResult | null {
  switch (name) {
    // C1: one payload builder, so both tools cannot drift apart.
    case "getCurrentSelection":
    case "getLatestSelection":
      return asText(selectionPayload(ctx.focusedFile()));
    case "getWorkspaceFolders": {
      const folders = ctx.workspaceFolders();
      return asText({
        success: true,
        folders: folders.map((folder) => ({
          name: path.basename(folder),
          uri: `file://${folder}`,
          path: folder,
        })),
        rootPath: folders[0], // the anchor is always first (B1, B6)
      });
    }
    case "getOpenEditors":
      return asText({ tabs: [] }); // C6

    // F2. Unadvertised, still called. Each answer is the honest one for a file
    // manager: nothing was open, so nothing was closed, saved, or diagnosed.
    case "closeAllDiffTabs":
      return asPlain("CLOSED_0_DIFF_TABS");
    case "close_tab":
      return asPlain("TAB_CLOSED");
    case "getDiagnostics":
      return asText([]);
    case "openDiff":
      return asPlain("DIFF_REJECTED");
    case "checkDocumentDirty":
    case "saveDocument":
      return asText({
        success: false,
        message: `Document not open: ${args.filePath}`,
      });

    // F3. The one out-of-scope tool yazi can perform.
    case "openFile": {
      const filePath = args.filePath;
      if (typeof filePath !== "string")
        return asText({
          success: false,
          message: "openFile requires filePath",
        });
      ctx.reveal(filePath);
      return asPlain(`Opened file: ${filePath}`);
    }

    default:
      return null;
  }
}
