# Yazi Claude Code IDE Plugin — PLAN

> Teach Claude Code's `/ide` to recognise yazi (a terminal file manager) the way it recognises VS Code or Neovim, so it can pull context such as the currently selected file. Status: tasks #1 and #2 complete, both gates passed. Task #3 not started.

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

## yazi-capability-spike result (2026-08-07, passed)

The second go/no-go gate **passed**. Full measurements are in [docs/yazi-capability.md](docs/yazi-capability.md) and [spike/yazi/README.md](spike/yazi/README.md).

**Decision: a sidecar is required, and the yazi plugin launches it.**

1. **A Lua plugin cannot own a long-lived child.** `Command:spawn()` returns a valid child and a nil error, then the process never runs — not even an instant `date >> file`. Holding the handle in a global changes nothing, so this is not handle-drop: the async plugin VM is destroyed when `entry` returns and takes its children with it. `:status()` and `:output()` work, because they block inside the same call. This kills the "Lua plugin owns the WebSocket server" option outright.
2. **A double-forked child survives, and survives too much.** `sh -c 'nohup … &'` launched via `:status()` outlives both a normal quit and `SIGKILL` of yazi. The sidecar must therefore terminate itself; nothing will do it.
3. **`ya sub` already carries the MVP payload, with no plugin involved.** Any process can run `ya sub hover,cd` and receive `{"tab":N,"url":"…"}` per event. Since the spike proved the MVP only needs a path, the plugin's one indispensable job is launching the sidecar — everything else can come off the DDS stream.
4. **`YAZI_ID` is inherited by plugin-spawned processes.** This is what makes the split work: `ya sub` is global across every yazi on the machine, and the sidecar can only filter its own instance's events because the plugin handed it `YAZI_ID`. A sidecar started any other way cannot.
5. **`ya emit-to` + `ya sub` make yazi fully drivable headlessly**, unlike the Claude Code side. `spike/yazi/harness.sh` scripts the whole loop. Task #6's checklist can be automated on the yazi half.

## workspace-policy probe result (2026-08-08)

Not a gate — a follow-up measurement that closes the one question task #1 left open. Details in [docs/baseline.md](docs/baseline.md), payloads in `spike/fixtures/session-60429.jsonl`.

1. **Adoption takes any matching entry in `workspaceFolders`, not the first one.** The lock file advertised `["/tmp/ws-decoy", "<repo root>"]` with `rootPath` also pointing at the decoy, and the session running in the repo root was still adopted. This is what makes a multi-entry policy possible at all.
2. **Workspace drift is not a running threat.** An adopted connection is never re-checked: the user can navigate anywhere in yazi and the session stays connected. Drift only bites at the moment `/ide` is pressed after navigating away — which is exactly what the anchor + cursor pair covers.

## Open questions (needed before implementation)

1. **How does selection map?** yazi has no cursor and no text selection, only a list of marked files and a currently focused file. Settled: the MVP recognises only the focused regular file (see MVP scope below).
2. **What are "open editors"?** yazi has no tabbed editor, only a preview pane. Settled: skipped in the MVP rather than guessing at semantics.
3. ~~**Implementation language and architecture:** deferred.~~ **Settled by task #2: a sidecar is required.** A Lua plugin cannot hold a process open past one `entry` call, so it cannot own the WebSocket server. The plugin double-forks the sidecar, which inherits `YAZI_ID`; the sidecar owns the lock file, the server, and the state stream. **Language settled: bun/TypeScript first, migrate to Rust once the design is stable.** `spike/fake-ide.ts` is already a working bun/TypeScript reference that completes the handshake with real Claude Code, so v1 grows from it rather than starting over. Rust is a later port, not a v1 requirement — the protocol is undocumented and still moving, and iteration speed matters more than binary size until it stops moving. The contract tests written in task #3 are what make the port checkable later.
4. **MVP scope:** settled as `getCurrentSelection` + `getWorkspaceFolders` + **the `selection_changed` push** (the push is required, or Claude receives nothing). Advertising only a few tools has been verified not to affect `/ide` adoption, so excluding the rest is safe. Benign responses are still needed for tools that go unadvertised but get called anyway — at minimum `closeAllDiffTabs`. **`text` stays empty: the push carries the path only.** Claude then reads the file with its own `Read` tool if it wants the contents. Task #1 measured both options as workable, so this is a product choice — it keeps the push cheap, avoids spending context on files the user merely scrolled past, and leaves reading under Claude's control rather than yazi's cursor.
5. **Are diagnostics meaningful here?** Settled: no. yazi is not an editor and has no LSP, so `getDiagnostics` and `openDiff` are excluded from v1 entirely.

## MVP semantic contract (draft)

- **Push `selection_changed` whenever focus changes.** This is the only channel that works. `getCurrentSelection` is still implemented in case Claude calls it, but measurement shows it will not.
- The payload for both `getCurrentSelection` and the push represents only the currently focused regular file's path, excluding marked files. Multi-select waits until there is a demonstrated need, so the first version does not stretch a single-selection method into unverified multi-file semantics.
- **`text` is always the empty string.** The push is a pointer, not a transfer. This also means the sidecar never reads user files, which keeps the whole integration to path-shaped data.
- When focus lands on a directory, or the file is missing or unreadable, return a well-defined empty or null result rather than raising a client error.
- ~~A workspace folder is defined as yazi's cwd at plugin startup.~~ **Superseded. Settled: `workspaceFolders` holds two entries, an anchor and a cursor.**
  - **Anchor** — the git root of the directory yazi started in, or that directory itself when it is not in a repo. Written once, never rewritten.
  - **Cursor** — yazi's current directory. Rewritten on every `cd`.
  - Adoption takes **any** entry that matches, so the pair covers both failure modes a single entry leaves open: the anchor catches a Claude session started at the project root while yazi sits deep in a subtree, and the cursor catches a session started in a monorepo subdirectory or a sibling project. Measured, see [docs/baseline.md](docs/baseline.md).
  - The cost is one extra lock-file entry. The rewrite mechanism itself was already measured working in task #2.
- Normalise all paths. Treat symlinks and files outside the workspace as valid input and return them directly, rather than designing unverified boundary rules up front.

## Architecture (settled by task #2)

```
yazi
  │  Lua plugin, on first invocation only:
  │  double-forks the sidecar, which inherits YAZI_ID
  ▼
sidecar process
  ▲  owns the lock file + WebSocket server + MCP dispatch
  │  ya sub hover,cd  ── filtered to its own YAZI_ID
  │
yazi DDS ── broadcasts hover/cd from every instance on the machine
  │
  ▼
Claude Code CLI (ws://127.0.0.1:<port>)
```

- The plugin's only indispensable job is launching the sidecar with `YAZI_ID` in scope. It cannot hold the process itself.
- The sidecar reads state off `ya sub`, filtering on the `sender` field. It never needs a private IPC channel for the MVP payload, because `hover` and `cd` already carry the path.
- The sidecar owns the lock file lifecycle, token generation, the WebSocket server, and responses to Claude's calls — the role `spike/fake-ide.ts` already fills.
- On `cd`, the sidecar rewrites the lock file's `workspaceFolders` **cursor entry only**, leaving the anchor entry alone. This is how the workspace-drift constraint (Known gaps #5) gets handled.
- **The sidecar must terminate itself.** A double-forked child outlives both a normal quit and `SIGKILL` of yazi, and DDS emits no departure event, so the sidecar has to poll for its yazi's absence.

The IPC hop the earlier draft assumed (unix socket, stdin/stdout) is not needed for the MVP. If the plugin later has to send something DDS does not carry — marked files, file contents — `ps.pub_to(0, "<kind>", table)` delivers an arbitrary Lua table to the sidecar's `ya sub`, measured working.

## Task breakdown (in order; the first two are go/no-go gates — stop if either fails)

1. ~~**protocol-spike**~~ — **complete, passed.** See "protocol-spike result" above.
2. ~~**yazi-capability-spike**~~ — **complete, passed.** See "yazi-capability-spike result" above.
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
- The sidecar exits after its yazi does, under both normal quit and `SIGKILL`, leaving no lock file behind.

The yazi half of this list can be automated: `ya emit-to` drives yazi and `ya sub` observes it, so `spike/yazi/harness.sh` scripts the whole loop headlessly. The Claude Code half cannot — `--ide` is interactive-only, per task #1.

## Known gaps

1. The protocol has no official specification, and details may shift with Claude Code releases — claudecode.nvim is itself continuously chasing new behaviour. See the compatibility baseline above.
2. ~~Whether the yazi plugin API can run a long-lived background process is unverified~~ **Answered: it cannot.** See the capability spike result above. The risk moved rather than closed — the sidecar now outlives yazi instead, and cleaning it up is `resilience-validation` work.
3. IDE semantics are inherently incomplete on yazi — no cursor, no LSP. This has to be accepted as a file-manager-grade integration, not a full IDE integration.
4. ~~Whether returning a path from `getCurrentSelection` is enough~~ **Answered:** the path does appear in the context (`The user opened the file <path> in the IDE.`), but the file contents do not. Claude does not read the file once it has the path. To get contents into the context, the plugin must read the file and fill `text`.
5. ~~The condition for `/ide` adopting a connection was not isolated~~ **Answered and closed.** The condition is that **some** entry in the lock file's `workspaceFolders` matches the Claude session's cwd. A successful socket handshake does not imply adoption. Neither the CLI's failure message (`IDE selection cancelled`) nor a session's own account of its state is trustworthy — only the IDE-side server log is. The mechanism (rewrite on `cd`) was measured in task #2; the policy (anchor + cursor) is settled in the MVP semantic contract above.

   Two loose ends remain, neither blocking the MVP. **When the CLI re-reads the lock file is unmeasured** — it certainly reads on connect, so the lock file has to stay in an acceptable state at all times. The anchor + cursor policy satisfies that by construction, which is why it costs nothing extra. **Whether a connected-but-unadopted socket gets adopted retroactively** once the lock file starts matching is also unmeasured; the MVP never depends on it, because the user reaches for `/ide` after navigating, not before.

6. **The sidecar outlives yazi and nothing cleans it up.** A double-forked child survives normal quit and `SIGKILL`, and DDS emits no departure event — no `bye`, and no refreshed `hey` roster when a peer leaves. The sidecar must poll for its yazi's absence. Belongs to `resilience-validation`.

7. **DDS server succession is untested.** The first yazi instance on the machine becomes the DDS server and later ones are clients. What happens to the surviving peers when the server instance exits was not measured, because testing it meant killing unrelated live yazi sessions. Affects the "two concurrent yazi instances" checklist item.

## Definition of done (MVP)

1. A pinned version of Claude Code discovers and connects to the yazi adapter through `/ide`.
2. After the focused regular file changes, Claude's context reflects the correct file. Note that per the spike, "reflects" means the path unless the plugin fills `text` with contents — the original wording of this criterion assumed contents arrive for free, and they do not.
3. Empty selection, directory focus, and unreadable files all produce well-defined responses that do not trigger a client error.
4. Normal exit, abnormal exit, and restart never leave a stale lock that blocks connections.
5. Two yazi instances can run at once without their tokens, ports, or lock files conflicting.
6. Every non-MVP tool (`openFile` and friends) has an explicit, client-verified response strategy — an empty result or not-supported, never undefined behaviour.
