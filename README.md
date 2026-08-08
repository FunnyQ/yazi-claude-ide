# yazi-claude-ide

Yazi plugin that speaks Claude Code's `/ide` protocol, so Claude Code can pull context (currently focused/selected file) from [yazi](https://yazi-rs.github.io/) the same way it does from VS Code or Neovim.

![Claude Code in one pane and yazi in another; Claude's prompt shows the file the yazi cursor sits on](assets/screenshot.png)

The sidecar is a single compiled Rust binary.

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

**Running two yazi instances in one repository?** The CLI auto-connects only
when one lock file matches, so `/ide` asks instead — and both rows read `yazi`
followed by the same repository path, because that path is the anchor and the
anchor is the same. Name the panes with `YCI_IDE_LABEL`:

```sh
export YCI_IDE_LABEL=api      # in one pane's yazi
export YCI_IDE_LABEL=web      # in the other's
```

The rows become `yazi (api)` and `yazi (web)`. Any string works; the sidecar
reads this one variable and nothing else, so if your terminal or multiplexer
already exports a per-pane identifier you can hand it that instead — 
`YCI_IDE_LABEL="$TMUX_PANE"` — and read the same variable in the pane you are
typing in to know which row is yours. Unset or blank, the name stays `yazi`.

The picker also ticks the connection the session already holds, which answers
the same question whenever the rows are distinguishable at all.

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
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
dev/manual/harness.sh verify                # the clauses only a real yazi can show
```

[`dev/docs/contract.md`](dev/docs/contract.md) is the specification. Every
automated test is named after the clause it covers, so `a5_server_binds_loopback_only`
answers to clause A5. Change behaviour by changing the clause first.

[`dev/docs/baseline.md`](dev/docs/baseline.md) and
[`dev/docs/yazi-capability.md`](dev/docs/yazi-capability.md) record the
measurements the clauses rest on. `dev/spike/` holds the tools that took them;
they run on bun and are not part of the build.

## License

MIT. See [LICENSE](LICENSE).
