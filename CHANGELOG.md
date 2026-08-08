# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.1] - 2026-08-09

_tracks tag `v0.2.1`_

### Added
- The plugin directory now ships a LICENSE and README.md, which `ya pkg` requires. This fixes `ya pkg add FunnyQ/yazi-claude-ide:claude-ide`, the install command in the README, which had never worked.

### Fixed
- `/ide` no longer fails with `Failed to connect to yazi.` on every attempt. Claude Code 2.1.226 sends `Sec-WebSocket-Protocol: mcp` on its upgrade request and disconnects if the server's `101 Switching Protocols` response doesn't echo it back; the sidecar had never echoed it, so every prior release (v0.1.0 and v0.2.0) was unusable with a real Claude Code. The sidecar now echoes any requested subprotocol.
- `serverInfo.version` reported a hardcoded `0.1.0`, left behind by the v0.2.0 bump. It now reads the crate's actual version at compile time.

## [0.2.0] - 2026-08-08

_tracks tag `v0.2.0`_

### Added
- Prebuilt binaries for Linux x86_64 and Linux arm64, statically linked against musl so they run on any distribution with no system libraries required.
- A `workflow_dispatch` trigger so the release build matrix can be exercised on demand, without cutting a tag.

### Changed
- `install.sh` now detects the platform (`uname -s`/`uname -m`) and installs the matching binary instead of assuming macOS, so the same one-line installer works on all three supported platforms.
- CI is split into three jobs: `check` (fmt, clippy, tests) now runs on both macOS and Linux; `build` compiles and verifies each of the three release targets; `publish` attaches all artifacts to the GitHub Release on a `v*` tag.
- README's setup section lists the three supported platforms and notes that the Linux builds are static.

## [0.1.0] - 2026-08-08

_tracks tag `v0.1.0`_

### Added
- First release: a `yazi` plugin that speaks Claude Code's `/ide` protocol, so Claude Code can pull editor context from yazi the way it does from VS Code or Neovim.
- A compiled Rust sidecar binary, double-forked by the Lua plugin, with no bun or TypeScript runtime dependency.
- A lock file at `<config>/ide/<port>.lock` that lets Claude Code discover the running sidecar.
- A loopback WebSocket server speaking MCP over JSON-RPC with header auth.
- Live cursor tracking: moving the selection in yazi pushes a `selection_changed` notification with the file path to Claude Code (never file contents).
- Marked-file mentions: a plugin keybinding publishes yazi's marked set to Claude Code as one `at_mentioned` notification per path, including directories.
- Install via `cargo install --path .` and a bare `require("claude-ide"):setup()` in yazi's config.
- A macOS arm64 CI and release workflow, and a test suite of 115 tests, each named after the specification clause it covers.
