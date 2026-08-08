# yazi-claude-ide

Yazi plugin that speaks Claude Code's `/ide` protocol, so Claude Code can pull context (currently focused/selected file) from [yazi](https://yazi-rs.github.io/) the same way it does from VS Code or Neovim.

The sidecar is a single compiled Rust binary. There is no TypeScript and no bun
at runtime. The plan, the protocol measurements, the spikes, and the manual
harness live in `dev/`, which is local-only and not published.

## Setup

The plugin and the sidecar binary it launches install separately.

**1. The plugin.**

```sh
ya pkg add FunnyQ/yazi-claude-ide:claude-ide
```

**2. The sidecar.**

```sh
curl -sSL https://raw.githubusercontent.com/FunnyQ/yazi-claude-ide/main/install.sh | bash
```

That puts the latest release in `~/.local/bin`; set `YCI_INSTALL_DIR` to choose
somewhere else. Prebuilt binaries cover macOS arm64, Linux x86_64, and Linux
arm64 — the Linux builds are statically linked against musl and need no system
libraries. On any other platform — or if you would rather read the source than
pipe it to a shell — build it:

```sh
cargo install --git https://github.com/FunnyQ/yazi-claude-ide
```

**3. The config**, in `~/.config/yazi/`:

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

Start yazi, then run `claude` in another pane — it finds the sidecar on its own.
Verified against Claude Code 2.1.226; the `/ide` protocol has no published
specification, so a newer CLI can change what it expects without notice.

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

## License

MIT. See [LICENSE](LICENSE).
