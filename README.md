# yazi-claude-ide

Yazi plugin that speaks Claude Code's `/ide` protocol, so Claude Code can pull context (currently focused/selected file) from [yazi](https://yazi-rs.github.io/) the same way it does from VS Code or Neovim.

Status: Discovery. The plan, the protocol measurements, the spikes, and the
manual harness live in `dev/`, which is local-only and not published.

## Setup

```sh
cargo install --path .
```

```lua
-- init.lua
require("claude-ide"):setup()
```

```toml
# keymap.toml — sends the marked files to Claude as @-mentions
[[mgr.prepend_keymap]]
on = ["c", "v"]
run = "plugin claude-ide"
desc = "Send the marked files to Claude"
```

Two channels, and they do different things:

- **Moving the cursor** tells Claude *which file you are looking at* — the path
  alone, no keypress, no contents.
- **Pressing `cv`** says *look at these*. Mark things with `space`, press `cv`,
  and each arrives as an `@` mention; submitting the prompt makes Claude read a
  file and list a directory. With nothing marked it sends whatever the cursor
  sits on, folders included.

The plugin never reads a file itself. Claude does, when you submit.

## Development

```sh
cargo test                                  # contract tests; each names the clause it covers
cargo clippy --all-targets -- -D warnings
cargo fmt --check
dev/manual/harness.sh verify                # the clauses only a real yazi can show
```

`dev/` is absent from a fresh clone, so contributors will not have the manual harness or contract.

`dev/docs/contract.md` is the specification.
