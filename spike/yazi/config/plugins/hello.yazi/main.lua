--- Process-capability probe. Each mode spawns a child a different way and
--- reports through ya.notify, because the plugin VM's own filesystem side
--- effects are part of what is being measured and cannot serve as the
--- reporting channel.
---
--- Bound to keys 1-7 by ../../keymap.toml. See ../../README.md for results.

local LOG = "/tmp/yazi-spike"

local function note(msg) ya.notify({ title = "probe", content = msg, timeout = 20 }) end

local M = {}

function M:entry(job)
	local mode = job.args[1]

	if mode == "spawn" then
		local child, err = Command("sh"):arg({ "-c", "date +%s >> " .. LOG .. "-spawn.log" }):spawn()
		note("spawn child=" .. tostring(child ~= nil) .. " err=" .. tostring(err))
	elseif mode == "status" then
		local st, err = Command("sh"):arg({ "-c", "date +%s >> " .. LOG .. "-status.log" }):status()
		note("status ok=" .. tostring(st and st.success) .. " err=" .. tostring(err))
	elseif mode == "output" then
		local out, err = Command("sh")
			:arg({ "-c", "echo hi; date +%s >> " .. LOG .. "-output.log" })
			:stdout(Command.PIPED)
			:output()
		note("output stdout=" .. tostring(out and out.stdout) .. " err=" .. tostring(err))
	elseif mode == "daemon" then
		local child, err = Command("sh")
			:arg({ "-c", "while :; do date +%s >> " .. LOG .. "-daemon.log; sleep 1; done" })
			:spawn()
		note("daemon child=" .. tostring(child ~= nil) .. " err=" .. tostring(err))
	elseif mode == "held" then
		-- Parks the handle in a global so dropping it cannot be what kills the
		-- child. It dies anyway; the plugin VM goes with the entry call.
		local child, err = Command("sh")
			:arg({ "-c", "while :; do date +%s >> " .. LOG .. "-held.log; sleep 1; done" })
			:spawn()
		HELD_CHILD = child
		note("held child=" .. tostring(child ~= nil) .. " err=" .. tostring(err))
	elseif mode == "detach" then
		-- Double-fork. The immediate child exits at once, orphaning the worker
		-- past anything yazi can reach. This is the only mode that survives.
		local st, err = Command("sh")
			:arg({
				"-c",
				"nohup sh -c 'while :; do date +%s >> "
					.. LOG
					.. "-detach.log; sleep 1; done' >/dev/null 2>&1 & echo $! > "
					.. LOG
					.. "-detach.pid",
			})
			:status()
		note("detach ok=" .. tostring(st and st.success) .. " err=" .. tostring(err))
	elseif mode == "env" then
		local out = Command("sh")
			:arg({ "-c", 'echo "YAZI_ID=${YAZI_ID:-unset} PPID=$PPID"' })
			:stdout(Command.PIPED)
			:output()
		note(tostring(out and out.stdout))
	else
		note("unknown mode=" .. tostring(mode))
	end
end

return M
