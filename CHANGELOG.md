# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
