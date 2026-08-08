--- Launches the sidecar that speaks Claude Code's /ide protocol. Launching it is
--- the plugin's only job: a plugin VM cannot hold a process open past one call,
--- so the sidecar is double-forked and inherits YAZI_ID to find its own events.
---
--- init.lua: require("claude-ide"):setup({ command = "bun /path/to/src/sidecar.ts" })
--- keymap.toml: run = "plugin claude-ide"   -- sends the marked files (H1)

local M = {}

--- H2. Reading and publishing both happen inside the hop, because both `cx` and
--- `ps` live only in the sync context — `entry()` runs in the async VM, where
--- `ps` is nil and touching it fails the whole plugin call. The marked set is
--- read on demand rather than watched: yazi announces nothing when it changes,
--- which is what H1 records.
local send_marked = ya.sync(function()
	local urls = {}
	for _, url in pairs(cx.active.selected) do
		urls[#urls + 1] = tostring(url)
	end
	ps.pub_to(0, "claude-marked", { urls = urls })
	return #urls
end)

function M:setup(opts)
	local command = (opts and opts.command) or "yazi-claude-ide"
	local log = "/tmp/yazi-claude-ide-" .. (os.getenv("YAZI_ID") or "unknown") .. ".log"

	-- No directory is passed: `cx` does not exist yet when init.lua runs setup(),
	-- and yazi's own cwd is not necessarily the directory it opened. The sidecar
	-- takes the directory from the first `cd` event instead, which yazi emits at
	-- startup.
	--
	-- Double fork. `spawn()` dies with the plugin VM and a foreground child would
	-- block yazi's startup, so the only shape that leaves a live process behind is
	-- nohup + `&` under a shell we wait for. That wait is instant. Measured in
	-- docs/yazi-capability.md.
	Command("sh")
		:arg({ "-c", "nohup " .. command .. " >> " .. ya.quote(log) .. " 2>&1 &" })
		:status()
end

--- H2. The user's "send these to Claude" gesture. An empty set is published as
--- an empty list rather than suppressed: the sidecar turns that into whatever
--- the cursor sits on (H7), the way yazi's own commands read an empty selection.
--- The sidecar decides, because only it knows the path still stats.
function M:entry()
	local n = send_marked()
	ya.notify({
		title = "claude-ide",
		content = n > 0 and string.format("sent %d marked item(s)", n) or "sent the item under the cursor",
		timeout = 3,
	})
end

return M
