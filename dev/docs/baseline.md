# Compatibility baseline

The protocol has no official specification. This document records what we observed at the time we checked, so that a future breakage can be attributed either to our own code or to an upstream change.

## Environment (2026-08-07)

| Item | Version |
| --- | --- |
| Claude Code | 2.1.223 |
| yazi | 26.5.6 (Homebrew 2026-05-05) |
| Node.js | v22.23.0 |
| Bun | 1.3.13 |
| macOS | Darwin 25.5.0 (arm64) |

## Protocol source

- Repo: [coder/claudecode.nvim](https://github.com/coder/claudecode.nvim)
- Pinned commit: `2390c6e45c4789072c293ac69de051d169668b29` (2026-06-25, `main`)
- Document: `PROTOCOL.md` at that commit
- Latest tag at the time: `v0.3.0`

`PROTOCOL.md` states that it was derived by reverse-engineering the VS Code extension. It is not an official specification.

## Protocol summary (per PROTOCOL.md)

Discovery:

- The IDE writes `~/.claude/ide/<port>.lock` with the fields `pid`, `workspaceFolders`, `ideName`, `transport: "ws"`, and `authToken`.
- `authToken` is 128 bits from a CSPRNG, rendered as 32 lowercase hex characters.
- When the IDE launches Claude it sets `CLAUDE_CODE_SSE_PORT` and `ENABLE_IDE_INTEGRATION=true`.
- The WebSocket server binds `127.0.0.1` only, on a port in the 10000-65535 range.

Authentication:

- Claude sends the header `x-claude-code-ide-authorization: <authToken>` during the WebSocket handshake.
- A mismatched header must be rejected.

Transport:

- **MCP (spec 2025-03-26) over WebSocket**, not bare JSON-RPC. The PLAN originally had this wrong.
- Names like `getCurrentSelection` are **MCP tool names** invoked through `tools/call`, not JSON-RPC method names.

IDE → Claude notifications:

- `selection_changed`: `{ text, filePath, fileUrl, selection: { start, end, isEmpty } }`
- `at_mentioned`: `{ filePath, lineStart, lineEnd }`

The 12 tools Claude may call:

`openFile`, `openDiff`, `getCurrentSelection`, `getLatestSelection`, `getOpenEditors`, `getWorkspaceFolders`, `getDiagnostics`, `checkDocumentDirty`, `saveDocument`, `close_tab`, `closeAllDiffTabs`, `executeCode`.

Naming: camelCase throughout, except `close_tab`.

## Measured: where PROTOCOL.md and Claude Code 2.1.223 disagree

Observed on 2026-08-07 by connecting `spike/fake-ide.ts` to a real Claude Code 2.1.223. **Where these conflict with PROTOCOL.md, trust these.**

1. **`protocolVersion` is `2025-11-25`, not the documented `2025-03-26`.**
2. **Claude sends an `ide_connected` notification** that PROTOCOL.md does not mention. Its payload is `{ "pid": <Claude's own pid> }`, it carries no `id`, and it needs no reply.
3. Claude's actual `initialize` params:

   ```json
   {
     "protocolVersion": "2025-11-25",
     "capabilities": { "roots": { "listChanged": true }, "elicitation": {} },
     "clientInfo": {
       "name": "claude-code",
       "title": "Claude Code",
       "version": "2.1.223",
       "description": "Anthropic's agentic coding tool"
     }
   }
   ```

4. **Connection order:** `initialize` → `notifications/initialized` → `ide_connected` → `tools/list` → `tools/call closeAllDiffTabs`.
   That trailing `closeAllDiffTabs` is a routine startup call, and **Claude makes it even when the tool was never advertised in `tools/list`**. Returning `-32601` does not drop the connection, but a benign result is safer. It also fires again on every subsequent `/ide` invocation, not just on the initial connect.
5. **Claude never calls `getCurrentSelection` on its own after connecting.** It stops after `tools/list`. So a file cannot reach the context by Claude pulling for it; the `selection_changed` push carries far more weight than expected.
6. A tool result is a **JSON string inside `content[0].text`**, not an object. It needs two levels of `JSON.parse`.

## Measured: what context injection actually does

After sending `selection_changed` with `text: ""` and only a `filePath`, exactly one line appears in the agent's context:

```
The user opened the file <absolute path> in the IDE.
```

**The path, and nothing else.** An agent in that session, barred from using tools, could not name a constant defined inside that file. So:

- Push a path → Claude knows which file you are looking at. **Verified working.**
- Want contents in the context → the plugin must read the file and fill `text`. Claude will not do it for you.

## Connected and adopted are two different states

Completing the WebSocket handshake does **not** mean the CLI treats the connection as the active IDE.

- Not adopted: the CLI prints `IDE selection cancelled`, yet the socket has completed a full handshake and stays open. Pushes are delivered and ignored.
- Adopted: the CLI prints `Connected to yazi.` and only then do pushes become context.
- **A session's own account of its connection state is not evidence.** It just repeats the CLI's message. The IDE-side server log is the only ground truth.

**Adoption requires the lock file's `workspaceFolders` to match the Claude session's cwd.**

Isolation experiment: workspace kept matching, `tools/list` cut from 11 tools to 4, `tools/call` handlers untouched → still `Connected to yazi.` So the advertised tool count has no bearing on adoption.

The converse also held: three earlier failures all had the workspace pointing at a scratchpad directory while the session's cwd was the repo root. The picker **still lists** the IDE; the rejection happens only after selecting it.

`IDE selection cancelled` covers at least two unrelated states, and the message alone cannot distinguish them:

| State | What the server sees |
| --- | --- |
| Workspace mismatch, adoption refused | Full handshake, socket stays open, pushes ignored |
| Already connected, picker simply dismissed | No new connection and no disconnect; the existing one keeps working |

## Which entry of `workspaceFolders` has to match (2026-08-08)

**Any entry. Not the first one, and not `rootPath`.**

Probe: the lock file advertised `workspaceFolders: ["/tmp/ws-decoy", "<repo root>"]`, and `getWorkspaceFolders` returned the same pair with `rootPath: "/tmp/ws-decoy"`. The Claude session ran in the repo root — matching only the second entry. It was adopted, and pushes landed in the context. See `spike/fixtures/session-60429.jsonl`.

This is what allows a multi-entry workspace policy: a fixed anchor plus a per-`cd` cursor, either of which can carry the match.

**Drift is not a running threat.** An adopted connection is never re-checked against the lock file. Navigating yazi anywhere — including outside every advertised folder — does not drop the connection or stop pushes from landing. The mismatch only matters at the moment `/ide` is pressed.

Two things left unmeasured, neither of them blocking:

- **When the CLI re-reads the lock file.** It reads on connect; whether it ever re-reads on a live connection was not tested. The safe assumption is that the lock file must be acceptable at all times.
- **Retroactive adoption.** A socket that handshook while the workspace did not match stays open and ignored. Whether it gets adopted later, once the lock file starts matching, was not tested.

## What the `/ide` picker shows, and what the tick means (2026-08-09)

With two sidecars anchored on the same repository the picker lists two rows, and
both read `yazi  <repo root>` — `ideName` and the anchor, and the anchor is the
same path for both. There is nothing to choose by. The CLI auto-connects only
when one lock file matches; two matches always ask.

Probe: one of the two live lock files had `ideName` rewritten to `yazi-PROBE-A`,
every other field untouched. That row's label changed to `yazi-PROBE-A`, both
rows stayed listed, and the already-adopted connection stayed adopted and kept
pushing. So `ideName` is **display only** — it reaches the picker label and the
`Connected to <ideName>.` line, and takes no part in matching, which stays on
`workspaceFolders` per the section above.

**The ✓ marks the connection the session already holds, not the cursor.** In the
same probe the cursor sat on row 2 while the ✓ stayed on row 1, and row 1 was
the instance whose `selection_changed` pushes were landing in that session. The
tick is therefore already the answer to "which one is mine" — it is only useless
when two rows are labelled identically, which is the case A3's `$YCI_IDE_LABEL`
suffix exists to break.

## Losing the IDE is silent (2026-08-08)

When the sidecar exits, a session that was adopted gets **no error and no
notification**. It simply stops receiving pushes. Pressing `/ide` afterwards
shows nothing, because the lock file is gone with the sidecar.

This is not a defect the sidecar can fix — the CLI owns that UX, and there is no
message in the protocol for "the IDE went away". It matters because clause G3
makes it routine: the sidecar exits whenever yazi does, so **every yazi quit
silently strips a connected session of its IDE context**.

The consequence for a user: reopening yazi starts a fresh sidecar with a new port
and a new token, so `/ide` has to be pressed again. An old session does not
reattach on its own.

Measured with a session that had been adopted and was receiving pushes, then
quitting yazi normally: the sidecar logged `yazi is gone, exiting`, removed its
lock file, and closed the socket; the session said nothing.

## Which IDE tools the agent can see

Advertise `getDiagnostics` and `mcp__ide__getDiagnostics` appears on the agent side; drop it and the tool disappears.

**That does not generalise.** Measured 2026-08-08 with all eleven tools advertised: `getDiagnostics` was the only one the CLI forwarded to the agent. `openFile`, `openDiff`, `saveDocument`, `checkDocumentDirty`, `close_tab`, and `closeAllDiffTabs` produced no `mcp__ide__*` tool, and neither did the four MVP tools the sidecar does advertise. So agent-side exposure is an allowlist **intersected with** `tools/list`, not `tools/list` itself. An earlier reading of this document generalised from the one tool that had been tested.

And **the advertised list does not constrain what the CLI itself calls** — `closeAllDiffTabs` and `openDiff` are both called whether or not they were advertised.

## Which tools a real CLI actually calls (2026-08-08)

Measured against the real sidecar over four `/ide` connections, logging every `tools/call`. Claude Code 2.1.223.

| Tool | Called | When |
| --- | --- | --- |
| `closeAllDiffTabs` | yes | on connect, and again around every diff |
| `openDiff` | yes | before every edit that needs confirming, including new files |
| `close_tab` | yes | twice per diff, same `tab_name` both times |
| `getDiagnostics` | yes | polled by the CLI, and via `mcp__ide__getDiagnostics` |
| `getOpenEditors` | **no** | never, not even during an edit |
| `checkDocumentDirty` | **no** | never |
| `saveDocument` | **no** | never |
| `openFile` | **no** | never |
| `getCurrentSelection`, `getLatestSelection`, `getWorkspaceFolders` | **no** | never — the workspace comes from the lock file, the selection from the push |

`getOpenEditors` staying uncalled was tested directly rather than assumed: the sidecar was made to claim one dirty open editor for the file about to be edited, and the CLI still never asked. So `checkDocumentDirty` and `saveDocument` are not gated on the open-editor answer; this CLI simply does not use them.

`openDiff`'s arguments are the full set — `old_file_path`, `new_file_path`, `new_file_contents`, and a `tab_name` like `✻ [Claude Code] .tool-probe.txt (5c8bea) ⧉`. The `close_tab` that follows carries the same `tab_name`.

## Answering `openDiff` cancels the edit (2026-08-08)

**The single most damaging finding so far, and it was live in the MVP.**

The CLI calls `openDiff` before every edit it needs confirmed, with only the four F1 tools advertised. Three responses, same edit, same file:

| Response to `openDiff` | What happened to the edit |
| --- | --- |
| `DIFF_REJECTED` | **cancelled** — the tool call came back as the user refusing |
| `DIFF_ACCEPTED` | applied |
| `-32601` (not implemented) | applied |

So the contract's original `DIFF_REJECTED` was not a benign placeholder. It reads to the CLI as the user rejecting the change, which means **every yazi user with `/ide` connected would find edits silently failing**. Reproduced twice, once with eleven tools advertised and once with four.

`DIFF_ACCEPTED` is not the fix: it claims the user approved a diff that was never displayed. `-32601` is what the contract now requires (F5) — it says truthfully that this IDE has no diff view, and puts the CLI back where it would be with no IDE attached.

### `-32601` keeps the prompt, but the CLI still claims yazi showed the diff

Measured with the confirmation prompt reaching a human — auto-approve disabled, and `openDiff` confirmed called in the sidecar log so the run is known to have taken the diff path:

```
Update(/workspace/.tool-probe.txt)

  Opened changes in yazi ⧉
  .tool-probe.txt
  Do you want to make this edit to .tool-probe.txt?
  > 1. Yes
    2. Yes, allow all edits during this session (shift+tab)
    3. No
```

**The user keeps the veto.** `-32601` does not skip confirmation, which is what separates it from `DIFF_ACCEPTED`.

**But the CLI prints `Opened changes in yazi`, and that is false** — the tool call was refused and yazi opened nothing. Worse, that line takes the place of the inline diff the CLI would otherwise print, so the user is asked to approve a change **they cannot see anywhere**: not in the terminal, because the CLI thinks the IDE has it, and not in yazi, which has no diff view.

The CLI evidently decides to defer the diff to the IDE from the mere fact that one is connected, before the `openDiff` result comes back — no answer the sidecar can give changes that line. This is the same class of problem as "losing the IDE is silent" above: the CLI owns the UX, and the protocol carries no way to say "I could not display this."

## Marked files reach the context through `at_mentioned` (2026-08-08)

Measured against Claude Code **2.1.226** — a later build than the 2.1.223 the rest of this document records — with `spike/fake-ide.ts` adopted by an interactive session.

`selection_changed` cannot carry a set: it is a single-slot state, and the CLI keeps only the last path. `at_mentioned` is the other notification PROTOCOL.md documents, and it accumulates. Two notifications, one per file, put both in the prompt:

```
❯ @PLAN.md @README.md ▊
```

- **One notification per file.** Nothing is overwritten, and yazi's order is preserved.
- **The prompt is not corrupted.** The mentions are inserted as text with the cursor left after them, so the user can keep typing or clear them.
- **The CLI shortens the path.** Absolute paths were sent; workspace-relative ones were displayed.
- **The range field decides whether this is a whole-file mention:**

  | params sent | rendered |
  | --- | --- |
  | `{filePath}` | `@PLAN.md` — the whole file |
  | `{filePath, lineStart: 0, lineEnd: 0}` | `@PLAN.md#L1` — a line anchor |
  | `{filePath, lineStart: 28, lineEnd: 29}` | `@CHANGELOG.md#L29-30` — a range |

  `0` is read as line one, not as "no range". A marked file in a file manager has no range, so **the fields must be omitted, not zeroed**.

  The third row was measured on 2026-08-09 against 2.1.226, through the whole
  section I path: a Neovim opened by yazi's block opener published lines 29-30,
  and the sidecar's 1-based-to-0-based conversion (I4) put `28, 29` on the wire.
  So the pair is 0-based and **inclusive** — `28, 29` is two lines, not three —
  and the render is `#L<first>-<last>` counting from 1. Until this measurement
  the range row was an inference from the `0, 0` row above it.

Untested and worth knowing before relying on this: whether `at_mentioned` is accepted while the prompt already holds text the user is typing, and whether the mention survives if the user never submits it.

### A mention delivers contents; a selection delivers only a path

Measured end to end on 2026-08-08 against the real sidecar and a real `/ide` session: marking two files in yazi and pressing the keybinding put `@tsconfig.json @package.json` in the prompt, and **submitting it made the CLI read both files into the context**.

This is the sharpest difference between the two channels, and it is easy to get backwards:

| Channel | What reaches the context |
| --- | --- |
| `selection_changed` | `The user opened the file <path> in the IDE.` — the path alone. The agent will not open it. |
| `at_mentioned` | the file's contents, fetched by the CLI when the prompt is submitted |

So the keybinding is not a multi-file version of the hover push. The push says *which file the user is looking at*; the mention says *read these*. The sidecar still never reads a user file itself (C4) — the CLI does it, on the user's submit.

### A directory mention works, and yields a listing

Measured 2026-08-08 by mentioning `src`, `docs`, and `README.md` in one batch — the file included as a control, so an empty prompt could not be confused with an unadopted connection. All three appeared: `@src @docs @README.md`, with no trailing slash and no distinction drawn between the kinds.

On submit the CLI **chose the operation from the path**: `ls` on each directory, `Read` on the file. So a mention has three behaviours, not two:

| Path mentioned | What Claude gets |
| --- | --- |
| a regular file | the file's contents |
| a directory | its listing |
| a path that does not stat | unmeasured, and no reason to send one |

This is why `at_mentioned` can carry what `selection_changed` must refuse. C5 and D5 keep directories out of the push because `filePath` there means an open editor, and a directory is not one. A mention has no such claim to break — it says "look at this", and the CLI knows how.

## Fixtures

See `spike/fixtures/`. One `.jsonl` per session, recording every message in both directions. Tokens, usernames, absolute paths, session ids, and pids are scrubbed; workspace paths are normalised to `/workspace`.
