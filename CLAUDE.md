# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```sh
cargo test                                  # whole suite
cargo test lock::                            # one module's unit tests
cargo test --test lifecycle                  # one integration target
cargo test a5_server_binds_loopback_only     # one test by name
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo install --path .                       # install the binary users' main.lua expects
dev/manual/harness.sh verify                 # the [manual] clauses, needs a real yazi
```

CI (`.github/workflows/ci.yml`) is three jobs: `check` runs `fmt`/`clippy`/`test` on `macos-15` and `ubuntu-24.04`, then runs `test` a second time with every environment variable `src/` reads set to a junk value; `build` compiles and packages a matrix of `aarch64-apple-darwin`, `x86_64-unknown-linux-musl`, and `aarch64-unknown-linux-musl`; `publish` attaches all three artifacts to the GitHub Release. It triggers on `v*` tags and on `workflow_dispatch`. Only `publish` is gated on the tag, so a dispatch run exercises the whole build matrix without cutting a version. Everything must stay green.

`install.sh` maps `uname` onto those same target triples and is served from the `main` raw URL, not from a tag. Change the asset names on one side and users are broken until the other side lands.

## The contract is the spec

`dev/docs/contract.md` defines clauses **A–I** and is the acceptance oracle for everything in `src/`. Read it before changing behaviour. Every automated test is named after the clause it covers (`a5_server_binds_loopback_only`, `e1_wrong_token_is_refused_with_401`, `d8_…`), so clause coverage is a grep.

Adding or changing behaviour means changing the contract first, then the test named for that clause.

**`dev/` is tracked, except `dev/PLAN.md`.** The contract, the measurements, the spikes, and the manual harness all ship in a fresh clone. `dev/PLAN.md` stays local because it is superseded and wrong — it still describes the push channel as `tokio::sync::broadcast`.

- `dev/docs/contract.md` — the A–I specification
- `dev/docs/baseline.md` — measurements behind F5, H4, the protocol version
- `dev/docs/yazi-capability.md` — the double-fork and `ya.sync` measurements behind G3, H2

Four clauses have no automated test, on purpose: **B7** is marked `[manual]` and only `harness.sh verify` reaches it; **H1** and **H2** govern the Lua plugin, which no Rust test can reach; and **I11** is a standing assumption about yazi, not about `src/` — `harness.sh verify` re-checks it against a real block opener.

## Architecture

`claude-ide.yazi/main.lua` (Lua) double-forks the `yazi-claude-ide` binary and exits. The binary is the **sidecar**, and it owns everything else. The two halves talk only through yazi's DDS event stream.

```
nvim ──ya pub-to 0──► "claude-editor-selection"         plugin/yazi-claude-ide.lua
  ▲
  └─ block opener, inherits YAZI_ID

yazi ──ya.sync──► ps.pub_to(0, "claude-marked")        claude-ide.yazi/main.lua
  │
  └─ double fork on setup(), inherits YAZI_ID
       ▼
     yazi-claude-ide
       main.rs    YAZI_ID guard · wiring · SIGINT/SIGTERM · shutdown
       yazi.rs    `ya sub hover,cd,claude-marked,claude-editor-selection,claude-diff-done` · `ya emit-to reveal` · `ya emit-to shell --block` · liveness poll
       lock.rs    <config>/ide/<port>.lock · anchor_for · workspace_folders · reclaim_stale
       server.rs  TcpListener 127.0.0.1:0 · accept_hdr_async · per-connection mpsc
         tools.rs   ADVERTISED · selection_payload · call_tool
       ▲
       │ ws://127.0.0.1:<port>, JSON-RPC 2.0
       │ x-claude-code-ide-authorization: <token>
   Claude Code CLI
```

| Module | Clauses | Tests |
| --- | --- | --- |
| `src/lock.rs` | A, B | `mod tests` in file |
| `src/tools.rs` | B6, C, E2, F, J1 | `mod tests` in file |
| `src/yazi.rs` | G2, G3, H3, I2, I3, I6, J3, J4 | `mod tests` in file |
| `src/server.rs` | A5, D, E, I5–I9, J5–J8 | `tests/server_rpc.rs`, `tests/server_push.rs` |
| `src/main.rs` | A6, B1, B3, G1–G3, J1–J3, J5, J8, J9 | `tests/lifecycle.rs` |

`tests/common/mod.rs` holds the async WebSocket client both server test files use. `tests/lifecycle.rs` drives the **compiled binary** via `env!("CARGO_BIN_EXE_yazi-claude-ide")` — it is the only check that proves the four modules compose, since the module tests call `start_sidecar` directly and cannot catch broken wiring.

### Three channels to Claude

Moving yazi's cursor pushes `selection_changed` — **the path alone, never contents**. Pressing the plugin's key publishes the marked set, which the sidecar fans out as one `at_mentioned` notification per path. The editor yazi's block opener runs publishes `claude-editor-selection` as the user drags a selection, which becomes a `selection_changed` carrying a range a file manager could never know (section I). From yazi the sidecar never reads a file; Claude does, when the user submits.

**`claude-editor-selection` is the one thing this sidecar forwards that is not a path.** Its `text` carries the lines the user selected, because the CLI counts its `N lines selected` display from the contents and not from `selection`. C4 is untouched: the editor already had those lines in a buffer, and no code path here opens a file. Do not "restore consistency" by emptying that field — but do understand what it buys, because the chip is the smaller half: with `text` present the agent's context receives the selected lines verbatim, with no submission and no mention, where an empty `text` yields only `The user opened the file <path> in the IDE.` Both halves are measured in [baseline.md](baseline.md), and I5 states the promise. The log line the same channel writes deliberately carries the range and **not** the text — that log lands in `/tmp`.

The third channel does not come from yazi and does not obey G2. An external `ya pub-to` is its own DDS peer, so `sender` names that `ya`, never the yazi the editor belongs to; and `ya pub-to <yazi id>` is refused outright, because a yazi accepts only kinds its own plugins subscribed to. Broadcast is the only route left, which puts the line in front of **every sidecar on the machine** — `yaziId` in the body is the whole of the addressing, and `dispatch` therefore branches on `claude-editor-selection` *before* the sender check. Both facts are measured; see clause I3.

**An earlier version of section I also carried a keypress gesture** that became an `at_mentioned` with a line range, rendering `@file#L10-20`. It worked and was deleted: the live selection already puts the same lines in front of the agent, so the gesture had no failure left to prevent. `git log` has it if the accumulating-mention behaviour is ever wanted back.

## Invariants that are easy to break

**Never hold a `Mutex` guard across an `.await`.** `server.rs` and `main.rs` both take the guard, read or mutate, clone out what the caller needs, and drop it before any await or file write. The runtime is `tokio` current-thread — one sidecar serves one yazi, so a held guard deadlocks rather than degrades.

**Push frames go through a per-connection `mpsc::UnboundedSender`, not a broadcast.** Unbounded is load-bearing: `mention()` pushes the whole marked set in one synchronous loop, and H5 requires order while H9 forbids drops. A bounded channel breaks both.

**`openDiff` is the only request answered out of band.** Every other `tools/call` is answered from `handle_json_rpc`'s return value, on the socket that asked. A configured viewer (section J) instead holds the request in `state.pending_diffs` and answers it later, from `finish_diff`, through that connection's `mpsc` sender — the same queue the pushes use. Two consequences a future reader will not guess: `handle_json_rpc` returning `None` now means *held*, not only *notification*; and the answer is `FILE_SAVED`, never `DIFF_ACCEPTED`, because the CLI's own prompt renders before any human can read a diff and an accept that arrives afterwards is measured to be ignored. J6 and [baseline.md](baseline.md) carry the measurement.

**`last_pushed` is one sidecar-wide value, not per-client.** Clause D8 reads as if it should be per-client, and the observable behaviour still satisfies it only because of the D3 exception: a connection-open push bypasses `push()` and writes straight to the joining socket. This is deliberate — do not "fix" it into per-client dedupe state.

**The anchor is provisional until the first `cd`.** yazi's cwd is where the user ran it, not necessarily where it opened, so `main.rs` seeds the anchor from `current_dir()` and latches the real one from the first `cd` event (which yazi emits at startup). That un-latched window lives for milliseconds and is what B1 describes.

**`YCI_DIFF_CMD` is off by default and section J does not run without it.** `YCI_POLL_MS` and `YCI_FAILURES_BEFORE_GONE` exist only for the lifecycle tests, to make liveness detection finish in about a second instead of production's measured six. Keep them in the `main.rs` wiring. Any new environment variable `src/` reads must also be added to the hostile-environment step in `ci.yml`, or that step stops covering it.

**Two `lock.rs` tests use this repository's own directory layout as a fixture.** Both build paths from `env!("CARGO_MANIFEST_DIR")` and point at `claude-ide.yazi/`. Renaming or moving a tracked top-level directory breaks `b1_the_anchor_is_the_git_root_or_the_directory_itself`: `anchor_for` shells out to `git -C <dir> rev-parse --show-toplevel`, and on a path that no longer exists git exits non-zero, so the function falls through to returning that path itself instead of the repo root. `b1_the_pair_is_anchor_then_cursor` keeps passing on the same stale path, because `workspace_folders` only builds strings and never touches disk. The same rename already cost the harness once: `dev/manual/config/plugins/claude-ide.yazi` is a tracked symlink into the plugin directory, it was left pointing at a `plugin/` that no longer exists, and the only symptom was `harness.sh start` reporting `started` while no sidecar ran. `cargo test` cannot see any of this.

**A green suite does not mean a real Claude Code can connect.** `tests/common/mod.rs` builds its upgrade request from the same assumptions as `server.rs`, so any requirement both sides are blind to survives every test. That is how E6 hid from the first version onwards: Claude Code sends `Sec-WebSocket-Protocol: mcp` and hangs up on a `101` that does not echo it, while 115 tests stayed green. `dev/spike/fake-ide.ts` was blind to it too — it runs on `Bun.serve`, whose `server.upgrade()` echoes the subprotocol for you, so every measurement in `baseline.md` was taken through that blind spot. To check a claim about the real client, put a logging TCP proxy in front of the sidecar and point `~/.claude/ide/<port>.lock` at the proxy; `fake-ide.ts` is the control that tells you whether a failure is ours or upstream.

**Two top-level plugin directories, for two different programs.** `claude-ide.yazi/` is the yazi plugin, installed by `ya pkg add`. `plugin/yazi-claude-ide.lua` is the Neovim plugin, installed by pointing any plugin manager at this repository — Neovim sources `plugin/*.lua` from the runtimepath root, which is why that file sits where it does and needs no `setup()`. Neither is part of the Rust build, and only the second one is the reference implementation of section I. An editor that is not Neovim replaces that file and nothing else.

**`claude-ide.yazi/` must keep its `LICENSE` and `README.md`.** They look redundant next to the ones at the repository root, and they are not: `plugin_files()` in yazi's `yazi-cli/src/package/dependency.rs` seeds a hardcoded `["LICENSE", "README.md", "main.lua"]` and never checks whether those files exist, so a missing one aborts the whole `ya pkg add` and deploys nothing. A root `LICENSE` does not satisfy it.

**Section I's two coordinate systems are not a mistake to tidy up.** `lineStart`/`lineEnd` on the wire are 1-based and inclusive and the sidecar subtracts one; `charStart`/`charEnd` are already 0-based with an exclusive end and pass through untouched. Making them agree breaks one of them. The character pair exists because the CLI's range is end-exclusive: a whole-line selection must end at the *length of the last line*, and a sidecar that filled that in would have to read the file (C4). Sending `0` there silently drops the last line from Claude's count and collapses a single-line selection to nothing — both measured, see [baseline.md](baseline.md).

**Logging is `eprintln!` to stderr.** `main.lua` creates `/tmp/yazi-claude-ide+<uid>/logs/` and redirects it to `<YAZI_ID>.log` inside it. The uid is in the name because /tmp is shared — `launch_diff` names its scratch root the same way. No logging framework.

**Failure paths mostly swallow and continue.** That is the contract's shape — the error surface stays small on purpose.

## Out of scope

Do not edit `dev/docs/contract.md` to make code pass. Do not change `claude-ide.yazi/main.lua` unless a clause in section H requires it. `dev/spike/` holds measurement tools that stay on bun and are not part of the build.
