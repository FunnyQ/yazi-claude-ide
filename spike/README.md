# protocol-spike

Answers one question: **if we hand Claude Code only a file path and no file contents, does that file reach its context?**

## Result (2026-08-07, Claude Code 2.1.223)

**Passed, but the answer differs from the assumption.**

- Claude **never** calls `getCurrentSelection` on its own. The `selection_changed` push is the only working channel.
- After pushing a path with `text: ""`, the context gains `The user opened the file <path> in the IDE.` — **the path only, no contents**.
- Claude does not read the file once it has the path. To get contents into the context, the plugin must read the file and fill `text` itself.
- `/ide` adopts a connection only when the lock file's `workspaceFolders` matches the Claude session's cwd. The number of advertised tools is irrelevant (`SPIKE_MINIMAL_TOOLS=1` advertises just 4 and still succeeds).
- **Any entry of `workspaceFolders` may carry the match** — not just the first, and not `rootPath`. Measured 2026-08-08 with a decoy first entry; see `fixtures/session-60429.jsonl`. Once adopted, the connection is never re-checked, so navigating yazi elsewhere does not break a live session.

See [../docs/baseline.md](../docs/baseline.md) for the full protocol differences and the connected-vs-adopted distinction.

## Files

| File | Purpose |
| --- | --- |
| `fake-ide.ts` | Minimal fake IDE. Writes the lock file, opens a WebSocket, implements the MCP handshake and the tools. |
| `probe.ts` | Test client that pretends to be Claude. Verifies auth, handshake, and tool responses. |
| `state.json` | The currently "selected" file. Editing it triggers a `selection_changed` push. Gitignored — it holds machine-local absolute paths. |
| `fixtures/` | Per-session record of every message in both directions. Tokens, usernames, paths, and pids are scrubbed. |

`fake-ide.ts` deliberately makes `getCurrentSelection` return an empty `text` — that is the spike's independent variable.

## Automated verification

```sh
bun spike/fake-ide.ts <workspace-dir...>   # in one terminal
bun spike/probe.ts                          # expect 10 passed, 0 failed
```

Passing more than one directory advertises them all in `workspaceFolders`; the first becomes `rootPath`. That is how the any-entry-matches result above was measured.

`probe.ts` verifies: lock file permissions, token format, wrong-token rejection, correct-token connection, `initialize`, `tools/list`, `getCurrentSelection`, `getWorkspaceFolders`, and `-32601` for an unimplemented tool.

Set `SPIKE_MINIMAL_TOOLS=1` to advertise only the four read-only tools while leaving `tools/call` behaviour unchanged. That is the isolation experiment that ruled out tool count as the adoption condition.

## Manual verification (required)

`claude -p` headless mode does **not** connect to an IDE; `--ide` only takes effect in interactive mode. A full `--debug-file` log contains no IDE discovery at all. So this last step must be run by a human.

1. Put a unique string in a target file and point `state.json` at it. Generate the string with the shell (`openssl rand -hex 6`) and never print it, so the agent under test provably has not seen it.
2. Terminal A: `bun spike/fake-ide.ts <workspace-dir...>` — at least one of the directories **must** match the cwd of the Claude session in step 3.
3. Terminal B: `cd <workspace-dir>` and run `claude`.
4. Run `/ide` and pick `yazi`. Expect `Connected to yazi.`
5. Ask: `Answer without using any tools. Which file am I looking at in the IDE, and what is the value of <the constant>?`

Reading the result:

| Outcome | Meaning |
| --- | --- |
| Names the path and the unique string | Path alone is enough; Claude reads the file itself. |
| Names the path but not the string | **This is what happens.** The path reaches the context; contents must be pushed in `text`. |
| Names neither | Check terminal A for `Claude connected`. No connection means discovery failed; a connection means the push was ignored. |

Terminal A's output shows which tools Claude actually called. That record is data regardless of the outcome.

## Manual verification: marked files via `at_mentioned` (open)

Answers the question that gates multi-select: **can more than one file reach the context at once?**

`selection_changed` cannot carry a set — it is a single-slot state, and pushing it N times leaves only the last path. `at_mentioned` is the only other channel PROTOCOL.md documents, and it has never been measured against a real CLI. yazi cannot help here either: its DDS kinds are `hi hey bye cd tab bulk load move yank hover mount trash moveItem delete rename download duplicate duplicateItem`, with **no marking event**, so the yazi side must be an explicit keybinding regardless of the outcome.

1. Terminal A: `bun spike/fake-ide.ts <workspace-dir>`.
2. Terminal B: `cd <workspace-dir>`, run `claude`, `/ide`, pick `yazi`. Expect `Connected to yazi.`
3. With the prompt empty and untouched, add two files to `state.json`:

   ```json
   { "filePath": "<workspace-dir>/PLAN.md", "mentions": ["<workspace-dir>/PLAN.md", "<workspace-dir>/README.md"] }
   ```

4. Watch the CLI's prompt line, then ask: `Answer without using any tools. Which files do you have in context right now?`

Reading the result:

| Outcome | Meaning |
| --- | --- |
| Both paths appear in the prompt or the context | Multi-select is buildable. `at_mentioned` is the channel. |
| Only one appears | The CLI holds a single mention slot, same as `selection_changed`. Multi-select is not buildable through this channel. |
| Neither appears, no error | `at_mentioned` is unimplemented in this CLI. Multi-select is not buildable at all. |
| The prompt is corrupted or the CLI errors | Record it — an IDE that can break the user's typing is worse than one that cannot multi-select. |

Also record whether the notification takes effect while the user is mid-typing, and whether `lineStart`/`lineEnd` of `0` is accepted for a whole-file mention.

Do not trust what the Claude session says about its own connection state — it only repeats the CLI's message. `IDE selection cancelled` is printed both when a workspace mismatch blocks adoption (the socket has completed a full handshake and stays open) and when an already-connected session simply dismisses the picker. The IDE-side server log is the only ground truth.
