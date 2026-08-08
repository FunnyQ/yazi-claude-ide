# yazi-claude-ide

Yazi plugin that speaks Claude Code's `/ide` protocol, so Claude Code can pull context (currently focused/selected file) from [yazi](https://yazi-rs.github.io/) the same way it does from VS Code or Neovim.

Status: Discovery. See [PLAN.md](PLAN.md) for scope, open questions, and task breakdown.

## Setup

```lua
-- init.lua
require("claude-ide"):setup({ command = "bun /path/to/src/sidecar.ts" })
```

```toml
# keymap.toml — sends the marked files to Claude as @-mentions
[[mgr.prepend_keymap]]
on = ["c", "v"]
run = "plugin claude-ide"
desc = "Send the marked files to Claude"
```

Moving the cursor tells Claude which file you are looking at, with no keypress.
The keybinding is for the other case: mark files with `space`, press `cv`, and
each one arrives as an `@file` mention in the prompt. With nothing marked it
sends the file under the cursor.

## Development

```sh
bun install
bun test                        # contract tests; each names the clause it covers
bun run typecheck               # tsc --noEmit; bun test is runtime-only
test/manual/harness.sh verify   # the clauses only a real yazi can show
```

[docs/contract.md](docs/contract.md) is the specification.
