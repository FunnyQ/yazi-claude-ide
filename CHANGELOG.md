# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.5.3] - 2026-08-22

_tracks tag `v0.5.3`_

### Changed
- Sidecar logs moved from `/tmp/yazi-claude-ide-<YAZI_ID>.log` to `/tmp/yazi-claude-ide+<uid>/logs/<YAZI_ID>.log`, and the diff viewer's scratch files from `<tmpdir>/yazi-claude-ide-diff-<token>/` to `<tmpdir>/yazi-claude-ide+<uid>/diff/<token>/`. Update anything that tails or greps the old log path; the plugin half only moves after `ya pkg upgrade`. The uid keeps both directories per-user on a machine where `/tmp` is shared.

## [0.5.2] - 2026-08-20

_tracks tag `v0.5.2`_

### Added
- A configured diff viewer: set `YCI_DIFF_CMD` and Claude's `openDiff` now launches it to show a proposed change before you accept, instead of silently applying edits with no preview. The request stays open until the viewer reports the file saved.
- `dev/spike/diff-client.ts`, a standalone tool for exercising the diff viewer path during development.

### Changed
- `dev/manual/harness.sh` and CI's hostile-environment matrix now cover `YCI_DIFF_CMD`, so a bad or unset value is verified rather than assumed.

## [0.5.1] - 2026-08-09

_tracks tag `v0.5.1`_

### Fixed
- A selection was undercounted by one line. Selecting lines 5 through 10 showed "5 lines selected" instead of 6, because the range Claude receives ends *before* the position it's given, and the editor stopped exactly at the start of the last line instead of past its end.
- Selecting a run of words or characters (`v` in Neovim) did nothing at all — only whole-line selections (`V`) registered. A charwise selection collapsed to zero width, so Claude had nothing to show.
- Dismissing a selection with Esc left the old indicator on screen, still reporting "1 line selected" after the selection was gone. It now clears back to the plain file indicator.

All three traced to the same root cause: the editor was only ever sending line numbers, never character positions. The DDS selection body gains two optional fields, `charStart` and `charEnd`, and the bundled Neovim plugin now computes them per visual mode; the README documents the wire format. An editor integration written against v0.5.0 keeps working without them, but it will still undercount by one line and can't represent a selection inside a single line. If you're on v0.5.0, update both the sidecar binary and the Neovim plugin — the fix spans both.

## [0.5.0] - 2026-08-09

_tracks tag `v0.5.0`_

### Added
- A third channel to Claude: an editor started by yazi's block opener (Enter on a file) can now publish your live selection back to the sidecar as you drag, and Claude shows it as an "N lines selected" indicator with the selected lines already in its context — no submission, no `@` mention. This is the only way a line range reaches Claude from this project, since a file manager has no line numbers of its own.
- The repository is now also an installable Neovim plugin (`{ "FunnyQ/yazi-claude-ide" }` in lazy.nvim, or `plugin/yazi-claude-ide.lua` copied by hand). It needs no setup call and no keybinding — selecting is the gesture, and it does nothing outside an editor that yazi opened. Any other editor can take its place by publishing `claude-editor-selection` over yazi's DDS; the README documents the wire format.

### Changed
- Claude counts its line display from the selected text, not from the range, so an editor that sends the range alone gets the plain file indicator: no line count, and nothing in Claude's context. Sending the text is what buys both. The sidecar still never opens a file itself — the content comes only from the editor's own buffer, and never covers more than what was selected by hand. The Neovim plugin drops the text above 100 KB and sends the range alone.

### Fixed
- The manual test harness had been broken since a directory rename left its plugin symlink dangling; it now points at the tracked `claude-ide.yazi/` directory again.
- The harness's teardown killed `ya sub` by matching its command line, which could silently stop *every* other yazi instance on the machine from pushing to Claude. It now targets only the process it started.

## [0.3.0] - 2026-08-09

_tracks tag `v0.3.0`_

### Added
- `YCI_IDE_LABEL` lets you tell apart multiple yazi instances open on the same repository. Set it and `/ide`'s picker shows `yazi (api)`, `yazi (web)`, and so on instead of two identical, unlabelled rows. Unset or blank, nothing changes. This does not make Claude Code auto-connect when several instances match a session's directory — the lock file still has no way to say "serve only this pane" — but it does make the manual picker usable.
- `dev/docs/` is now part of the repository: the A–H contract every test is named after, plus the protocol and yazi-capability measurements behind it. A fresh clone now ships its own specification.

### Fixed
- `install.sh` now warns if another `yazi-claude-ide` earlier on `PATH` would shadow the copy it just installed, instead of silently reporting success while yazi keeps running the stale binary.
- `.gitignore`'s `docs/` rule matched at any depth and was silently excluding `dev/docs/`; it's now anchored to the repository root.
- The README's build-from-source command now installs to `--root ~/.local`, so it no longer diverges from the other supported install path.

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
