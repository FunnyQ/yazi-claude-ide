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
- **F5.** With no viewer configured (J1), `openDiff` MUST return `-32601`, and MUST NOT be answered. It is the one tool where a benign-looking answer does damage: the CLI calls it before every edit that needs confirming — in the F1 four-tool list, not only when advertised — and reads `DIFF_REJECTED` as **the user rejecting the change**, so the edit is silently cancelled. `DIFF_ACCEPTED` is worse: it asserts an approval for a diff the user was never shown. `-32601` says what is true, that this IDE has no diff view, and **is measured to keep the CLI's own confirmation prompt**, so the user still holds the veto. Measured against 2.1.223 — see [baseline.md](baseline.md), which also records the residual cost no answer can avoid: the CLI announces `Opened changes in yazi` and suppresses its inline diff, so the change is approved unseen. Section J is what closes that hole, and it does so **without taking this clause's answer back** — a configured viewer still leaves the veto where F5 puts it.

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

## I. The editor's live selection

Sections C and H carry paths without ranges, because a file manager has no ranges. The editor yazi's block opener runs does have them, and it is a separate process sitting on the far side of that opener. This section is the only path by which a line range reaches Claude.

The route is measurable from outside yazi and every clause below was settled that way on 2026-08-09, against Ya 26.5.6 and Claude Code 2.1.226 with a real block opener holding the terminal.

- **I1.** A live selection is state, not a gesture. The editor MAY publish it as the selection changes, with no keypress, and the sidecar MUST NOT ask the editor for it. **Dismissing the selection is itself a publish**: an editor that goes quiet when the user presses `Esc` leaves the CLI displaying a selection that no longer exists, which is the editor-side twin of the staleness I8 prevents on the yazi side. It publishes a zero-width range instead (I5). It writes to the single slot section D owns, and I7 and I8 are what let it do so without fighting D4 and D6. An editor that publishes nothing costs the user nothing: sections C, D, and H are unaffected.
- **I2.** The editor MUST publish the kind `claude-editor-selection` with `ya pub-to 0`, and the sidecar MUST subscribe to it alongside `hover`, `cd`, and `claude-marked`. Broadcast is the only route there is: `ya pub-to <yazi id>` and `ya pub` are both refused with ``Cannot send message: Receiver `<id>` does not have the ability to receive `claude-editor-selection` messages``, because a yazi instance accepts only the kinds its own plugins subscribed to, and the plugin of H2 subscribes to none.
- **I3.** The body MUST be `{yaziId, url, lineStart, lineEnd}` with optional `charStart`, `charEnd`, and `text`, and the sidecar MUST filter it on `yaziId`, **not** on `sender` as G2 requires of every other kind. `ya pub-to` joins DDS as a peer in its own right and stamps `sender` with a fresh id of its own, ignoring the `$YAZI_ID` in its environment. So a sidecar filtering on `sender` drops every selection, and — because I2 forces a broadcast that reaches every sidecar on the machine — one filtering on nothing pushes the file into unrelated sessions. The editor learns `yaziId` from the `$YAZI_ID` it inherited from yazi.
- **I4.** **The two coordinates do not share a convention, and this is the clause to read before touching either.** `lineStart` and `lineEnd` are 1-based and inclusive, the way editors count lines, and the sidecar converts them to the 0-based pair the CLI reads — `line: 0` is the first line (H4). `charStart` and `charEnd` are already 0-based with an **exclusive** end, and the sidecar passes them through untouched. They differ because the character pair is not a count of anything: `charEnd` is the offset the selection stops before, and for a whole-line selection that offset is the length of the last line — a number the sidecar cannot know, because knowing it means reading the file (C4). The editor is the only party that can supply it.

  Both default to `0` when absent, which is a legal selection and a wrong one: measured 2026-08-09, lines 5 through 10 sent with `charEnd: 0` displayed as `5 lines selected` rather than `6`, because ending at character 0 of the last line excludes that line entirely. An editor that omits the pair therefore undercounts by one line, and a single-line selection collapses to zero width. See [baseline.md](baseline.md).
- **I5.** For each live selection the sidecar MUST push `selection_changed` shaped as D1 and D2 require, with `selection` carrying the converted 0-based lines and the character pair verbatim, and `text` set to whatever `text` the editor published — `""` when it published none.

  `isEmpty` MUST be derived, never asserted: it is `true` exactly when the range is zero-width — same line, same character — and `false` otherwise. A zero-width publish MUST also force `text` to `""`, whatever the editor sent, because a selection that covers nothing has no contents by definition and the CLI counts its display from that text. This is how a dismissed selection returns the display to the plain file indicator, and it is the same shape D2 already pushes for a yazi hover. **This is the one push where `text` is not empty, and D2's reference to C2 does not reach it.** Measured: a frame with the range filled in, `isEmpty: false`, and `text: ""` was accepted and drew `In CLAUDE.md` — the file chip, not a range — so the CLI computes its `N lines selected` display from the contents and not from `selection`. C4 is unchanged and still binding: the sidecar MUST NOT read a file to fill this field. The contents come from the editor, which already had them in a buffer, and only ever cover what the user selected by hand. **What `text` does is larger than the display it was added for**: the agent's context receives `The user selected the lines 7 to 14 from <path>:` followed by those lines verbatim, where an empty `text` produces only `The user opened the file <path> in the IDE.` Selecting in the editor therefore puts those lines in front of the agent with no submission and no mention. That is the promise this clause makes, and an editor that does not want to make it MUST omit `text`, at the cost of the line count. An editor SHOULD omit it above a size of its own choosing: selecting a whole file is one keystroke, and the alternative is pushing that file through DDS and the WebSocket to drive a line count. See [baseline.md](baseline.md) for both measurements.
- **I6.** `url` MUST be an absolute unresolved path (B5) naming a regular file that stats — C5's test, not H6's, because a range over a directory means nothing. A body missing a required field, carrying a non-numeric line, a line below 1, or `lineStart` greater than `lineEnd` MUST be dropped whole and MUST NOT be repaired. A non-numeric or negative character offset MUST be dropped the same way, and when `lineStart` equals `lineEnd`, `charStart` greater than `charEnd` MUST be dropped too — on one line that is a reversed selection, while across lines it is ordinary. A missing `text`, `charStart`, or `charEnd` is none of those: each is optional, and I4 says what omitting the pair costs. E5 applies throughout — a malformed publish MUST NOT be able to kill the sidecar.
- **I7.** A live selection MUST NOT be deduped, so D6 does not apply to it. Dragging a selection publishes range after range for one unchanged path, and a path-keyed dedupe would freeze the CLI on whichever range arrived first.
- **I8.** After pushing a live selection the sidecar MUST forget `last_pushed`, so the next yazi `hover` pushes even when it lands on the file the editor was just in. Without it, quitting the editor and moving around yazi leaves the CLI displaying a selection the user has already left.
- **I9.** With no connection open a live selection MUST be dropped and MUST NOT be queued (D7, H8), and every open connection MUST receive it (D8). Unlike D3, no joining connection is owed one: the editor may well have exited by then, and a stale range is worse than none.
- **I10.** Every accepted selection MUST be logged — the path and the range, and **never the `text`**, which would put the contents of the user's files in `/tmp`. This channel has no other observable: it produces no yazi UI, it happens behind a block opener, and its whole failure mode is silence. The log line is also what `dev/manual/harness.sh verify` greps for, so it is the only reason I2, I3, and I11 can be checked against a real yazi at all.
- **I11.** yazi MUST keep routing DDS while the block opener holds the terminal, or the channel does not exist. Verified rather than assumed: with a single yazi owning its own `.dds.sock` — the worst case, since every message routes through the blocked process — a publish issued while the opener was up was delivered to an outside subscriber. This clause is a standing assumption about yazi, not a requirement on the sidecar; `harness.sh verify` re-checks it.

## J. The diff viewer

F5 answers `openDiff` with `-32601` and keeps the user's veto in the CLI. It cannot keep the
user's *eyes* there: the CLI suppresses its inline diff the moment an IDE is connected and prints
`Opened changes in yazi` instead, so the change is approved unseen. This section opens a viewer in
yazi's own pane. It does not move the veto, and clause J6 says why it must not try.

Measured 2026-08-20 against Claude Code 2.1.235; see [baseline.md](baseline.md) for all six runs.

- **J1.** The viewer is opt-in and has no default. `$YCI_DIFF_CMD` holds a shell command; unset or
  blank means section J does not run and F5 stands unchanged. The command receives the two paths as
  `$1` and `$2` — the user's file and the sidecar's copy of the proposed contents, in that order.
  `nvim -d "$1" "$2"` is the value that makes J5 mean something; a read-only viewer such as
  `git diff --no-index --color=always -- "$1" "$2" | delta` is equally legitimate and simply yields
  no amendment.
- **J2.** The sidecar MUST write `new_file_contents` to a file it owns, mode `0600`, and MUST pass
  that path as `$2`. It MUST NOT read the user's file to build the diff — C4 is unchanged, and the
  viewer is the party that reads both sides.
- **J3.** The sidecar MUST run the template through its own yazi —
  `ya emit-to <id> shell <template> --block` — because only yazi can hand the terminal over. The
  sidecar is double-forked and has no terminal of its own.
- **J4.** The viewer's exit MUST reach the sidecar as the DDS kind `claude-diff-done`, body
  `{yaziId, token}`, published with `ya pub-to 0` and filtered on `yaziId` exactly as I3 requires —
  `ya pub-to` stamps a `sender` of its own, so G2's filter would drop it. `token` names which
  `openDiff` is being answered; a body naming an unknown token MUST be dropped.
- **J5.** On `claude-diff-done` the sidecar MUST answer the held `openDiff` with `FILE_SAVED` and
  the contents of `$2` **as they stand at that moment**, then delete the file. Whatever the user
  changed in the viewer is what Claude writes. Measured: a `FILE_SAVED` sent twenty seconds late
  still had its bytes written, and one that arrived after the user had already approved in the CLI
  was discarded rather than applied — so both orderings are safe and neither can clobber the file
  behind the user.
- **J6.** The sidecar MUST NOT answer `DIFF_ACCEPTED` or `DIFF_REJECTED`. The CLI renders its own
  confirmation prompt immediately after calling `openDiff`, and an answer only suppresses that
  prompt if it beats the render — measured, an accept at `t+20s` was acknowledged with `close_tab`
  and left the prompt live. **Any diff a human is given time to read answers too late to hold the
  veto**, so a sidecar that claimed it would be asserting an approval it cannot deliver. `FILE_SAVED`
  is the one verdict that carries the user's work without claiming their consent.
- **J7.** Every failure MUST fall back to F5. A missing `$YAZI_ID`, a template that will not spawn,
  a write that fails, a `claude-diff-done` that never comes, or a connection that closes first MUST
  leave the request answered with `-32601` or unanswered — never with a fabricated verdict. The CLI
  is measured not to time out an unanswered `openDiff` (242 s, no retry), so an abandoned request
  costs the user nothing but the prompt they already have.
- **J8.** The log line MUST carry the path and the tab name and **never the contents**, as I10
  requires of the editor channel. The proposed contents are the user's file in all but name, and
  the log lands in `/tmp`.
- **J9.** A viewer that edits the user's file directly — `nvim -d` writes the left buffer too — is
  the user editing their own file, and the sidecar MUST NOT try to detect or prevent it. Only `$2`
  is read back.

## Non-goals for the MVP

Recorded so that a future reader can tell a deliberate omission from an oversight.

- File contents in `text` for the yazi channels (C4) and in a range mention (I5). Claude reads files with its own `Read` tool; from yazi the sidecar sends where to look, never what is there. The editor's live selection is the deliberate exception and the only one — see I10, which says what it costs.
- Diagnostics, dirty state, saving — yazi cannot answer any of them honestly (F2). Diffs left this list on 2026-08-20: section J answers `openDiff` with a viewer yazi can actually run.
- Retroactive adoption of a socket that connected while the workspace did not match. Unmeasured, and the MVP does not depend on it.
