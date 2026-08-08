# claude-ide.yazi

Sends the file under your cursor, or the files you have marked, to a running
Claude Code session.

This directory holds only the Lua plugin. The plugin launches a separate sidecar
binary that speaks Claude Code's `/ide` protocol, and that binary is **not**
installed by `ya pkg` — install it separately.

## Install

```sh
ya pkg add FunnyQ/yazi-claude-ide:claude-ide
curl -sSL https://raw.githubusercontent.com/FunnyQ/yazi-claude-ide/main/install.sh | bash
```

Then, in `~/.config/yazi/init.lua`:

```lua
require("claude-ide"):setup()
```

And in `~/.config/yazi/keymap.toml`:

```toml
[[mgr.prepend_keymap]]
on  = ["c", "v"]
run = "plugin claude-ide"
```

Full setup notes, supported platforms, and the source live at
<https://github.com/FunnyQ/yazi-claude-ide>.
