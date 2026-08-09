# MVP contract

The testable form of the MVP semantic contract. Every clause is numbered so a test can name the clause it covers. `MUST`, `MUST NOT`, and `MAY` carry their RFC 2119 meaning.

Clauses marked **[manual]** cannot be automated: `claude --ide` only takes effect in interactive mode, per [baseline.md](baseline.md). Everything else is automatable — against `probe.ts` on the Claude side, and against `ya emit-to` on the yazi side.

## Vocabulary

| Term | Meaning |
| --- | --- |
| **sidecar** | The process the yazi plugin double-forks. Owns the lock file, the WebSocket server, and the `ya sub` stream. |
| **focused file** | The single file yazi's cursor sits on — yazi's `hover` event. Not the marked-file set. |
| **marked files** | The set yazi's `cx.active.selected` holds — what the user built with `space`. Reaches Claude only through section H. |
| **anchor** | The git root of yazi's startup directory, or that directory itself when it is not in a repository. Fixed for the sidecar's lifetime. |
| **cursor folder** | yazi's current directory. Changes on every `cd`. |
| **adopted** | The CLI printed `Connected to yazi.` — distinct from a completed WebSocket handshake. |
| **editor** | The program yazi's block opener runs on `Enter`. A separate process that inherits `$YAZI_ID`. Reaches Claude only through section I. |

## A. Discovery and the lock file

- **A1.** The sidecar MUST write `<config>/ide/<port>.lock`, where `<config>` is `$CLAUDE_CONFIG_DIR` when set and `~/.claude` otherwise.
- **A2.** The lock directory MUST be mode `0700` and the lock file mode `0600`.
- **A3.** The lock file MUST contain exactly the fields `pid`, `workspaceFolders`, `ideName`, `transport`, `authToken`. `transport` MUST be `"ws"`. `ideName` MUST be `"yazi"`, except that when `$YCI_IDE_LABEL` is set and not blank it MUST be `"yazi (<label>)"` with the label's surrounding whitespace trimmed. `ideName` is display only: measured 2026-08-09 by renaming one of two live lock files, the CLI's `/ide` picker rendered the new value as that row's label, listed and adopted the connection unchanged, and left the already-adopted connection connected. Two yazi instances anchored on the same repository are otherwise indistinguishable in that picker — both rows carry `ideName` and the anchor, and the anchor is the same path.
- **A4.** `authToken` MUST be 128 CSPRNG bits rendered as 32 lowercase hex characters, generated once per sidecar.
- **A5.** The server MUST bind `127.0.0.1` only. The port is whatever the OS assigns; the lock file name MUST match the bound port.
- **A6.** The sidecar MUST delete its lock file on `SIGINT`, `SIGTERM`, and on self-termination.
- **A7.** On startup the sidecar MUST remove any lock file in the directory whose `pid` no longer exists. It MUST NOT touch lock files belonging to live processes, including other yazi instances.

## B. Workspace folders

- **B1.** `workspaceFolders` MUST hold the anchor followed by the cursor folder. The anchor MUST be derived from the directory in the first `cd` event, not from the sidecar's own working directory — `cx` is unreadable when the plugin starts, and yazi's process cwd is not necessarily the directory it opened.
- **B2.** When anchor and cursor folder are the same path, `workspaceFolders` MUST hold one entry, not a duplicate pair.
- **B3.** On every `cd` event the sidecar MUST rewrite the lock file so the second entry is the new cursor folder, and MUST leave the anchor entry unchanged. The sidecar holds the anchor in memory and republishes both entries; the lock file is not the source of truth for which entry is the anchor.
- **B4.** The rewrite MUST preserve `port`, `pid`, and `authToken`. Rewriting MUST NOT restart the server or drop a live connection.
- **B5.** Every entry MUST be an absolute path with no trailing slash. Symlinks MUST NOT be resolved — the path yazi reports is the path advertised.
- **B6.** `getWorkspaceFolders` MUST return the same set as the lock file, shaped `{success: true, folders: [{name, uri, path}], rootPath}`, with `name` the basename, `uri` the `file://` form, and `rootPath` the anchor.
- **B7.** **[manual]** With the anchor matching and the cursor folder pointing elsewhere, `/ide` MUST adopt the connection; and with the cursor folder matching and the anchor elsewhere, `/ide` MUST also adopt. Measured against the spike client first — see `spike/fixtures/session-60429.jsonl` — and on 2026-08-08 against the real implementation and a real `/ide`: yazi opened in `~/Projects/q-lab`, which is not a repository, so the anchor stayed there; navigating into `~/Projects/q-lab/yazi-claude-ide` moved only the cursor entry, and `/ide` from a session rooted in that repository printed `Connected to yazi.` The **cursor** entry earned that adoption, which is the single-entry failure mode the pair exists to prevent.

## C. Selection payloads

- **C1.** `getCurrentSelection` and `getLatestSelection` MUST return the same payload shape.
- **C2.** With a focused regular file the payload MUST be `{success: true, filePath, text, selection}` where `text` is `""` and `selection` is a zero-width range at line 0, character 0 with `isEmpty: true`.
- **C3.** `filePath` MUST be the absolute path of the focused file, unresolved (B5).
- **C4.** `text` MUST always be `""`. The sidecar MUST NOT read the contents of user files.
- **C5.** With no focused regular file — nothing focused, a directory focused, or a path that no longer stats — the payload MUST be `{success: false, message: "No active editor found"}`. This is a successful JSON-RPC result, not an error (E2).
- **C6.** `getOpenEditors` MUST return `{tabs: []}`. yazi has no editor tabs and the MVP does not invent them.

## D. The `selection_changed` push

- **D1.** The sidecar MUST send `selection_changed` as a JSON-RPC notification — `method` and `params`, no `id`.
- **D2.** `params` MUST be `{text, filePath, fileUrl, selection}`, with the same values as C2 and `fileUrl` the `file://` form of `filePath`.
- **D3.** The sidecar MUST push once after the connection is established, so a session that connects mid-navigation sees the current file.
- **D4.** The sidecar MUST push whenever the focused file changes to a different regular file.
- **D5.** The sidecar MUST NOT push when focus lands on a directory, and MUST NOT push a path that does not stat. The previously pushed file stands.
- **D6.** The sidecar MUST NOT push the same `filePath` twice in a row.
- **D7.** With no connection open, a focus change MUST be recorded and MUST NOT be queued for replay. The next connection gets one push of the then-current file (D3).
- **D8.** More than one client MAY be connected at once. D3 is owed to each connection separately — a client that joins while another already holds the current path MUST still be pushed that path — while D6 applies per client. A focus change MUST reach every open connection.

## E. Error semantics

- **E1.** A WebSocket upgrade whose `x-claude-code-ide-authorization` header is missing or does not equal `authToken` MUST be refused with HTTP `401`, and MUST NOT be upgraded.
- **E2.** A `tools/call` for a tool the sidecar does not implement MUST return JSON-RPC error `-32601`. An unknown `method` MUST likewise return `-32601`. No other condition in the MVP produces a JSON-RPC error.
- **E3.** A frame that does not parse as JSON MUST be logged and dropped. The sidecar MUST NOT reply, because no `id` can be recovered.
- **E4.** An incoming message with no `id` is a notification and MUST NOT be answered.
- **E5.** The sidecar MUST survive every clause above without exiting. A malformed client MUST NOT be able to kill it.
- **E6.** An authorized upgrade that carries a `Sec-WebSocket-Protocol` header MUST echo that header's value in the `101` response. Claude Code 2.1.226 requests `mcp` and closes the connection the instant the response omits it, which the CLI reports as `Failed to connect to yazi.` The value is echoed verbatim; selecting one subprotocol from a comma-separated offer is out of scope while the only client sends exactly one.

## F. Tools that are out of scope but still called

Measured: the CLI calls tools it was never offered — `closeAllDiffTabs` on every connection, and `openDiff` before every edit that needs confirming. So implementing a tool and advertising it are separate decisions, and **a tool being unadvertised is no reason to think its answer is unreachable**.

- **F1.** `tools/list` MUST advertise exactly `getCurrentSelection`, `getLatestSelection`, `getWorkspaceFolders`, and `getOpenEditors`. `getDiagnostics` and `openDiff` MUST NOT be advertised — yazi has no LSP and no diff view, and advertising `getDiagnostics` puts `mcp__ide__getDiagnostics` in front of the agent, which can only lie. Advertising the rest changes nothing on the agent side: measured 2026-08-08 with all eleven tools advertised, the CLI forwarded only `getDiagnostics` to the agent, so F1 governs one tool's visibility rather than seven.
- **F2.** Every tool below MUST return the stated response rather than `-32601`, whether or not it was advertised:

  | Tool | Response |
  | --- | --- |
  | `closeAllDiffTabs` | `CLOSED_0_DIFF_TABS` |
  | `close_tab` | `TAB_CLOSED` |
  | `getDiagnostics` | `[]` |
  | `checkDocumentDirty` | `{success: false, message: "Document not open: <path>"}` |
  | `saveDocument` | `{success: false, message: "Document not open: <path>"}` |

- **F3.** `openFile` MUST reveal the file in yazi — `ya emit-to <id> reveal <path>` — and return `Opened file: <path>`. This is the one out-of-scope tool yazi can honestly perform.
- **F4.** Any tool not named in F1–F3 MUST return `-32601` (E2). Silence and `undefined` are both forbidden.
- **F5.** `openDiff` MUST return `-32601`, and MUST NOT be answered. It is the one tool where a benign-looking answer does damage: the CLI calls it before every edit that needs confirming — in the F1 four-tool list, not only when advertised — and reads `DIFF_REJECTED` as **the user rejecting the change**, so the edit is silently cancelled. `DIFF_ACCEPTED` is worse: it asserts an approval for a diff the user was never shown. `-32601` says what is true, that this IDE has no diff view, and **is measured to keep the CLI's own confirmation prompt**, so the user still holds the veto. Measured against 2.1.223 — see [baseline.md](baseline.md), which also records the residual cost no answer can avoid: the CLI announces `Opened changes in yazi` and suppresses its inline diff, so the change is approved unseen.

## G. Lifecycle

- **G1.** At most one sidecar MUST exist per yazi instance. Launching from the plugin's `setup()`, which yazi runs once per instance, satisfies this without a guard.
- **G2.** The sidecar MUST filter `ya sub` events on `sender`, acting only on its own `YAZI_ID`. It MUST ignore an event whose `url` is absent, empty, or JSON `null` — all three occur.
- **G3.** The sidecar MUST poll for its yazi's absence and exit once yazi is gone. DDS emits no departure event, and a double-forked child survives both a normal quit and `SIGKILL` of yazi.
- **G4.** Two concurrent yazi instances MUST end up with distinct ports, tokens, and lock files, and neither sidecar may delete the other's lock file.

## H. Marked files and the `at_mentioned` push

Section D carries one file and replaces it on every move. This section carries a set, once, when the user asks for it. The two never interfere: `selection_changed` is a single-slot state, so a set cannot ride it.

- **H1.** Sending marked files MUST be an explicit user gesture, bound to a key. yazi emits no DDS event when the marked set changes — its Ember kinds are `hi hey bye cd tab bulk load move yank hover mount trash moveItem delete rename download duplicate duplicateItem`, with nothing for marking — so a sidecar cannot mirror the set and MUST NOT try.
- **H2.** The plugin's `entry()` MUST read `cx.active.selected` and publish the paths with `ps.pub_to(0, "claude-marked", …)`, and **both MUST happen inside one `ya.sync` hop**. `entry()` runs in the async VM, where `ps` is `nil` and reaching for it fails the whole plugin call — silently, with no notification and nothing on the wire. Publishing from `entry()` itself therefore satisfies a looser reading of this clause and ships a plugin that does nothing. See [yazi-capability.md](yazi-capability.md) for the failure and the only place it is visible.
- **H3.** The sidecar MUST subscribe to `claude-marked` alongside `hover` and `cd`, and MUST filter it on `sender` like every other kind (G2).
- **H4.** For each path in the set the sidecar MUST send `at_mentioned` as a JSON-RPC notification — `method` and `params`, no `id` — with `params` exactly `{filePath}`. `lineStart` and `lineEnd` MUST be omitted. Measured 2026-08-08 against 2.1.226: omitting them renders `@PLAN.md`, while sending `0` for both renders `@PLAN.md#L1`, a line anchor a marked file never meant. The omission is scoped to the marked set — I5 is the case where a range exists and is sent. See [baseline.md](baseline.md).
- **H5.** `filePath` MUST be absolute and unresolved (B5), and the notifications MUST go out in the order yazi lists the set.
- **H6.** A directory MUST be mentioned like any other path. Measured 2026-08-08: the CLI reads a mentioned file and lists a mentioned directory, choosing by what the path is. Only a path that does not stat MUST be skipped — this is deliberately **not** C5's test, which exists to keep directories out of `selection_changed`, where `filePath` claims an open editor. A mention makes no such claim.
- **H7.** With the marked set empty the gesture MUST fall back to the path under the cursor, matching how yazi's own commands treat selected-or-hovered. That path is whatever yazi last hovered, **including a directory** — it is not the focused file of section C, which is file-only by C5 and would make the fallback silent exactly when the user is standing on a folder. The sidecar MUST therefore track the hovered path separately from the focused file. With neither, it MUST send nothing.
- **H8.** With no connection open the gesture MUST send nothing and MUST NOT queue for replay, as in D7. Unlike D3, no connection is owed a set it missed — the gesture is the user's, not the sidecar's.
- **H9.** Every open connection MUST receive the set (D8). There is no dedupe: pressing the key twice MUST send twice, deliberately unlike D6, because a repeat is the user asking again.

## I. Editor line ranges and the range mention

Section H carries paths without ranges, because a file manager has no ranges. The editor yazi's block opener runs does have them, and it is a separate process sitting on the far side of that opener. This section is the only path by which a line range reaches Claude.

The route is measurable from outside yazi and every clause below was settled that way on 2026-08-09, against Ya 26.5.6 with a real block opener holding the terminal.

- **I1.** Sending a range MUST be an explicit user gesture in the editor, bound to a key, as in H1. The sidecar MUST NOT ask the editor for its selection, and the editor MUST NOT publish on cursor movement. The editor is not a second `selection_changed` source; section D owns that slot and D6 would fight it.
- **I2.** The editor MUST publish the kind `claude-selection` with `ya pub-to 0`, and the sidecar MUST subscribe to it alongside `hover`, `cd`, and `claude-marked`. Broadcast is the only route there is: `ya pub-to <yazi id>` and `ya pub` are both refused with ``Cannot send message: Receiver `<id>` does not have the ability to receive `claude-selection` messages``, because a yazi instance accepts only the kinds its own plugins subscribed to, and the plugin of H2 subscribes to none.
- **I3.** The body MUST be `{yaziId, url, lineStart, lineEnd}` and the sidecar MUST filter it on `yaziId`, **not** on `sender` as G2 requires of every other kind. `ya pub-to` joins DDS as a peer in its own right and stamps `sender` with a fresh id of its own, ignoring the `$YAZI_ID` in its environment. So a sidecar filtering on `sender` drops every range, and — because I2 forces a broadcast that reaches every sidecar on the machine — one filtering on nothing mentions the file into unrelated sessions. The editor learns `yaziId` from the `$YAZI_ID` it inherited from yazi.
- **I4.** `lineStart` and `lineEnd` in the DDS body MUST be 1-based and inclusive, the way editors count lines. The sidecar MUST convert them to the 0-based pair the CLI renders — `lineStart: 0` renders `#L1` (H4) — so that the off-by-one lives in one tested place instead of in every editor's config.
- **I5.** The sidecar MUST send `at_mentioned` as a JSON-RPC notification with `params` exactly `{filePath, lineStart, lineEnd}`. This is the one case H4's omission does not cover: an editor selection really does have a range, which is the whole reason this channel exists. **How a real CLI renders a non-zero pair is not yet measured** — H4 measured only `0, 0`, which rendered `#L1`, and `#L10-20` for `9, 19` is an inference from it. `/ide` is interactive-only (B7), so settling this needs a human and it belongs in [baseline.md](baseline.md) once taken.
- **I6.** `url` MUST be an absolute unresolved path (B5) naming a regular file that stats — C5's test, not H6's, because a range over a directory means nothing. A body missing a field, carrying a non-numeric line, a line below 1, or `lineStart` greater than `lineEnd` MUST be dropped whole and MUST NOT be repaired. E5 applies to every one of them: a malformed publish MUST NOT be able to kill the sidecar.
- **I7.** With no connection open the gesture MUST send nothing and MUST NOT queue for replay (H8), every open connection MUST receive it (H9, D8), and repeating the gesture MUST send again. There is no dedupe — a repeat is the user asking again.
- **I8.** yazi MUST keep routing DDS while the block opener holds the terminal, or the channel does not exist. Verified rather than assumed: with a single yazi owning its own `.dds.sock` — the worst case, since every message routes through the blocked process — a publish issued while the opener was up was delivered to an outside subscriber. This clause is a standing assumption about yazi, not a requirement on the sidecar; `dev/manual/harness.sh verify` re-checks it.

## Non-goals for the MVP

Recorded so that a future reader can tell a deliberate omission from an oversight.

- File contents in `text` (C4), and the selected text in a range mention (I5). Claude reads files with its own `Read` tool; the sidecar sends where to look, never what is there.
- Diagnostics, diffs, dirty state, saving — yazi cannot answer any of them honestly (F2).
- Retroactive adoption of a socket that connected while the workspace did not match. Unmeasured, and the MVP does not depend on it.
