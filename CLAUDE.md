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

CI (`.github/workflows/ci.yml`) runs all three on macOS arm64 and publishes a binary on a `v*` tag. All three must stay green.

## The contract is the spec

`dev/docs/contract.md` defines clauses **A–H** and is the acceptance oracle for everything in `src/`. Read it before changing behaviour. Every automated test is named after the clause it covers (`a5_server_binds_loopback_only`, `e1_wrong_token_is_refused_with_401`, `d8_…`), so clause coverage is a grep.

Adding or changing behaviour means changing the contract first, then the test named for that clause.

**`dev/` is gitignored.** A fresh clone has no contract, no measurements, no manual harness. If `dev/` is absent you cannot verify a behaviour claim — say so rather than guessing.

- `dev/docs/contract.md` — the A–H specification
- `dev/docs/baseline.md` — measurements behind F5, H4, the protocol version
- `dev/docs/yazi-capability.md` — the double-fork and `ya.sync` measurements behind G3, H2

Three clauses have no automated test, on purpose: **B7** is marked `[manual]` and only `harness.sh verify` reaches it; **H1** and **H2** govern the Lua plugin, which no Rust test can reach.

## Architecture

`claude-ide.yazi/main.lua` (Lua) double-forks the `yazi-claude-ide` binary and exits. The binary is the **sidecar**, and it owns everything else. The two halves talk only through yazi's DDS event stream.

```
yazi ──ya.sync──► ps.pub_to(0, "claude-marked")        main.lua
  │
  └─ double fork on setup(), inherits YAZI_ID
       ▼
     yazi-claude-ide
       main.rs    YAZI_ID guard · wiring · SIGINT/SIGTERM · shutdown
       yazi.rs    `ya sub hover,cd,claude-marked` · `ya emit-to reveal` · liveness poll
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
| `src/tools.rs` | B6, C, E2, F | `mod tests` in file |
| `src/yazi.rs` | G2, G3, H3 | `mod tests` in file |
| `src/server.rs` | A5, D, E | `tests/server_rpc.rs`, `tests/server_push.rs` |
| `src/main.rs` | A6, B1, B3, G1–G3 | `tests/lifecycle.rs` |

`tests/common/mod.rs` holds the async WebSocket client both server test files use. `tests/lifecycle.rs` drives the **compiled binary** via `env!("CARGO_BIN_EXE_yazi-claude-ide")` — it is the only check that proves the four modules compose, since the module tests call `start_sidecar` directly and cannot catch broken wiring.

### Two channels to Claude

Moving yazi's cursor pushes `selection_changed` — **the path alone, never contents**. Pressing the plugin's key publishes the marked set, which the sidecar fans out as one `at_mentioned` notification per path. The sidecar never reads a file; Claude does, when the user submits.

## Invariants that are easy to break

**Never hold a `Mutex` guard across an `.await`.** `server.rs` and `main.rs` both take the guard, read or mutate, clone out what the caller needs, and drop it before any await or file write. The runtime is `tokio` current-thread — one sidecar serves one yazi, so a held guard deadlocks rather than degrades.

**Push frames go through a per-connection `mpsc::UnboundedSender`, not a broadcast.** Unbounded is load-bearing: `mention()` pushes the whole marked set in one synchronous loop, and H5 requires order while H9 forbids drops. A bounded channel breaks both. (`dev/PLAN.md` still says `tokio::sync::broadcast`; the code is right and the plan is stale.)

**`last_pushed` is one sidecar-wide value, not per-client.** Clause D8 reads as if it should be per-client, and the observable behaviour still satisfies it only because of the D3 exception: a connection-open push bypasses `push()` and writes straight to the joining socket. This is deliberate — do not "fix" it into per-client dedupe state.

**The anchor is provisional until the first `cd`.** yazi's cwd is where the user ran it, not necessarily where it opened, so `main.rs` seeds the anchor from `current_dir()` and latches the real one from the first `cd` event (which yazi emits at startup). That un-latched window lives for milliseconds and is what B1 describes.

**`YCI_POLL_MS` and `YCI_FAILURES_BEFORE_GONE` exist only for the lifecycle tests**, to make liveness detection finish in about a second instead of production's measured six. Keep them in the `main.rs` wiring.

**Two `lock.rs` tests use this repository's own directory layout as a fixture.** Both build paths from `env!("CARGO_MANIFEST_DIR")` and point at `claude-ide.yazi/`. Renaming or moving a tracked top-level directory breaks `b1_the_anchor_is_the_git_root_or_the_directory_itself`: `anchor_for` shells out to `git -C <dir> rev-parse --show-toplevel`, and on a path that no longer exists git exits non-zero, so the function falls through to returning that path itself instead of the repo root. `b1_the_pair_is_anchor_then_cursor` keeps passing on the same stale path, because `workspace_folders` only builds strings and never touches disk.

**Logging is `eprintln!` to stderr.** `main.lua` redirects it to `/tmp/yazi-claude-ide-<YAZI_ID>.log`. No logging framework.

**Failure paths mostly swallow and continue.** That is the contract's shape — the error surface stays small on purpose.

## Out of scope

Do not edit `dev/docs/contract.md` to make code pass. Do not change `claude-ide.yazi/main.lua` unless a clause in section H requires it. `dev/spike/` holds measurement tools that stay on bun and are not part of the build.
