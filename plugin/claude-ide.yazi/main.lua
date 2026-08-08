--- Launches the sidecar that speaks Claude Code's /ide protocol. Launching it is
--- the plugin's only job: a plugin VM cannot hold a process open past one call,
--- so the sidecar is double-forked and inherits YAZI_ID to find its own events.
---
--- init.lua: require("claude-ide"):setup({ command = "bun /path/to/src/sidecar.ts" })

local M = {}

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

return M
