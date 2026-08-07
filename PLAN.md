# Yazi Claude Code IDE Plugin — PLAN

> Teach Claude Code's `/ide` to recognise yazi (a terminal file manager) the way it recognises VS Code or Neovim, so it can pull context such as the currently selected file. Status: task #1 complete, task #2 not started.

## Context

Claude Code exchanges context with IDEs — selection, open files, diagnostics — over a protocol that is undocumented but has been reverse-engineered by the community. Third-party ports exist for Neovim, Emacs, Eclipse, IntelliJ, and Obsidian; there is no yazi version on GitHub. yazi is a pure terminal file manager with no editor semantics — no cursor, no LSP — but it does have a notion of the currently selected file or directory, which is worth evaluating against the IDE interface Claude Code expects.

## Protocol summary (verified)

- **Discovery:** the IDE writes a lock file to `~/.claude/ide/<port>.lock` (or `$CLAUDE_CONFIG_DIR/ide/`) holding the WebSocket port, a 128-bit random token, the IDE process id, the IDE name, and the workspace folders. File mode `0600`, directory mode `0700`.
- **Connection:** the IDE listens on `ws://127.0.0.1:<port>` (loopback only, no TLS). The Claude CLI reads the lock file for the port and token, then connects with the header `X-Claude-Code-Ide-Authorization: <token>`.
- **Message format:** MCP (spec 2025-03-26) over WebSocket transport, with JSON-RPC 2.0 underneath. Not bare JSON-RPC.
- **Core tools:** `getCurrentSelection`, `getOpenEditors`, `getWorkspaceFolders`, `getDiagnostics`, `openFile`, `openDiff`, `saveDocument`, `checkDocumentDirty`. These are **MCP tool names** invoked through `tools/call`, not JSON-RPC method names. The IDE must implement `initialize` and `tools/list` first.
- **IDE → Claude notifications:** `selection_changed` (carrying `text`, `filePath`, `fileUrl`, `selection`) and `at_mentioned`. Pushing is a separate path from Claude calling a tool.
- **Best precedent:** [coder/claudecode.nvim](https://github.com/coder/claudecode.nvim) — it has a `PROTOCOL.md`, and its architecture is the closest to yazi's non-traditional-editor situation.

Source: Claude Code's official documentation does not publish the protocol. The details above come from claudecode.nvim's PROTOCOL.md and related reverse-engineering notes.

**Compatibility baseline:** established, see [docs/baseline.md](docs/baseline.md). Pins claudecode.nvim commit `2390c6e4`, Claude Code 2.1.223, yazi 26.5.6. Measured payload fixtures live in `spike/fixtures/`.

## Codex review (2026-08-07)

PLAN.md was handed to codex for review (`relay:relay codex review`). Conclusion: the direction is sound, but two core assumptions had locked in an architecture and a definition of done without ever being tested, and had to become verifiable go/no-go gates before implementation. The sections and task order below were rewritten accordingly.

## protocol-spike result (2026-08-07, passed)

The go/no-go gate for task #1 **passed**, but the answer differs from the original assumption. Full measurements are in [docs/baseline.md](docs/baseline.md) and [spike/README.md](spike/README.md).

Four findings that overturned the original assumptions:

1. **Claude never calls `getCurrentSelection` on its own.** After connecting it performs `tools/list` and then goes silent. The `selection_changed` push is the only way to get state in — it is mandatory, not optional.
2. **Pushing a path reaches the context; pushing contents does not.** After a `selection_changed` with `text: ""`, the context gains `The user opened the file <path> in the IDE.`, but Claude does not go and read that file. To put contents in the context, the plugin must read the file and fill `text` itself.
3. **`/ide` adopts a connection based on the workspace matching, not on the tool set.** Completing the WebSocket handshake does not mean the CLI treats the connection as the active IDE. When the lock file's `workspaceFolders` does not match the Claude session's cwd, the picker still lists the IDE, but selecting it prints `IDE selection cancelled` — **while the socket has in fact completed a full handshake and stays open**, with pushes delivered and ignored. The isolation experiment (workspace matching, advertised tools cut from 11 to 4, handlers unchanged) produced `Connected to yazi.`, proving the tool count is irrelevant.
4. **The MCP tools the CLI forwards to the agent track `tools/list`.** Advertising `getDiagnostics` makes `mcp__ide__getDiagnostics` appear on the agent side; dropping it makes the tool disappear. But Claude still calls tools that were never advertised — `closeAllDiffTabs` is called on every connection — so the advertised list governs what the agent can see, not what the CLI will call.

## Open questions (needed before implementation)

1. **How does selection map?** yazi has no cursor and no text selection, only a list of marked files and a currently focused file. Settled: the MVP recognises only the focused regular file (see MVP scope below).
2. **What are "open editors"?** yazi has no tabbed editor, only a preview pane. Settled: skipped in the MVP rather than guessing at semantics.
3. **Implementation language and architecture:** deferred. Whether a Rust sidecar is needed depends on `yazi-capability-spike` (task #2) determining whether a yazi Lua plugin can manage a long-lived process and its IPC on its own. If it can, no sidecar.
4. **MVP scope:** settled as `getCurrentSelection` + `getWorkspaceFolders` + **the `selection_changed` push** (the push is required, or Claude receives nothing). Advertising only a few tools has been verified not to affect `/ide` adoption, so excluding the rest is safe. Benign responses are still needed for tools that go unadvertised but get called anyway — at minimum `closeAllDiffTabs`. Whether `text` carries file contents is a product choice, not a technical constraint.
5. **Are diagnostics meaningful here?** Settled: no. yazi is not an editor and has no LSP, so `getDiagnostics` and `openDiff` are excluded from v1 entirely.

## MVP semantic contract (draft)

- **Push `selection_changed` whenever focus changes.** This is the only channel that works. `getCurrentSelection` is still implemented in case Claude calls it, but measurement shows it will not.
- The payload for both `getCurrentSelection` and the push represents only the currently focused regular file's path, excluding marked files. Multi-select waits until there is a demonstrated need, so the first version does not stretch a single-selection method into unverified multi-file semantics.
- When focus lands on a directory, or the file is missing or unreadable, return a well-defined empty or null result rather than raising a client error.
- A workspace folder is defined as yazi's cwd at plugin startup. Settled here rather than left to implementation; revisit only if the current tab's cwd proves necessary.
- Normalise all paths. Treat symlinks and files outside the workspace as valid input and return them directly, rather than designing unverified boundary rules up front.

## Architecture draft (pending spike, not locked)

```
yazi (Lua plugin)
  │  selection state changes (yazi sync/event API)
  ▼
sidecar process (Rust or a light script)
  │  owns the lock file + WebSocket server + JSON-RPC dispatch
  ▼
Claude Code CLI (ws://127.0.0.1:<port>)
```

- The Lua plugin detects yazi selection changes and syncs state to the sidecar over a unix socket or stdin/stdout.
- The sidecar owns the lock file lifecycle (write on start, delete on exit), token generation, the WebSocket server, and responses to Claude's calls.
- This split is itself an untested hypothesis, not a decision. Whether a sidecar is genuinely needed depends on the outcome of `yazi-capability-spike` (task #2).

## Task breakdown (in order; the first two are go/no-go gates — stop if either fails)

1. ~~**protocol-spike**~~ — **complete, passed.** See "protocol-spike result" above.
2. **yazi-capability-spike** — verify whether a yazi Lua plugin can start and manage a long-lived child process, covering tab switching, plugin reload, and the lifecycle when yazi exits normally or abnormally. Confirm that focus, marked files, current tab, and cwd can be read reliably. Must also verify whether the lock file can be rewritten live as yazi's cwd changes (see Known gaps #5). Output: the architectural decision on whether a sidecar is needed.
3. **contract** — turn the MVP semantic contract draft above into a testable specification: payload shapes, empty-value behaviour, workspace definition, error semantics.
4. **protocol-core** — implement the lock file lifecycle, auth, WebSocket, and JSON-RPC dispatch, with contract tests.
5. **yazi-binding** — wire up the focused file and workspace state, using a sidecar or not depending on the outcome of task #2.
6. **resilience-validation** — see the verification checklist below.

## Verification checklist (beyond the happy path)

- Lock directory and file modes are `0700` and `0600` respectively.
- The server binds loopback only, and a wrong token cannot connect.
- Two concurrent yazi instances do not overwrite each other's lock file, port, or token.
- After the sidecar crashes or is killed, the next startup recognises and reclaims the stale lock.
- Both orderings work: Claude Code started first, and yazi started first.
- The connection recovers after a WebSocket drop, and returns fresh data once yazi's state changes.
- `/ide` finds yazi and the context updates after selecting a file. This was the original happy-path manual test; it is kept, but it is not the only verification.

## Known gaps

1. The protocol has no official specification, and details may shift with Claude Code releases — claudecode.nvim is itself continuously chasing new behaviour. See the compatibility baseline above.
2. Whether the yazi plugin API can run a long-lived background process is unverified, and is the largest technical risk `yazi-capability-spike` must answer.
3. IDE semantics are inherently incomplete on yazi — no cursor, no LSP. This has to be accepted as a file-manager-grade integration, not a full IDE integration.
4. ~~Whether returning a path from `getCurrentSelection` is enough~~ **Answered:** the path does appear in the context (`The user opened the file <path> in the IDE.`), but the file contents do not. Claude does not read the file once it has the path. To get contents into the context, the plugin must read the file and fill `text`.
5. ~~The condition for `/ide` adopting a connection was not isolated~~ **Answered:** the condition is that the lock file's `workspaceFolders` matches the Claude session's cwd. A successful socket handshake does not imply adoption. Neither the CLI's failure message (`IDE selection cancelled`) nor a session's own account of its state is trustworthy — only the IDE-side server log is. **This is a real constraint for yazi:** yazi's cwd changes as the user navigates, while the lock file's workspace is fixed at startup. When and how those drift apart has to be handled in `yazi-binding`.

## Definition of done (MVP)

1. A pinned version of Claude Code discovers and connects to the yazi adapter through `/ide`.
2. After the focused regular file changes, Claude's context reflects the correct file. Note that per the spike, "reflects" means the path unless the plugin fills `text` with contents — the original wording of this criterion assumed contents arrive for free, and they do not.
3. Empty selection, directory focus, and unreadable files all produce well-defined responses that do not trigger a client error.
4. Normal exit, abnormal exit, and restart never leave a stale lock that blocks connections.
5. Two yazi instances can run at once without their tokens, ports, or lock files conflicting.
6. Every non-MVP tool (`openFile` and friends) has an explicit, client-verified response strategy — an empty result or not-supported, never undefined behaviour.
