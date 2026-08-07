--- Measures what a yazi plugin can read and what it can hand to an outside
--- process. Reports over ya.notify (in-TUI) and ps.pub_to (to `ya sub`),
--- because the plugin VM's own filesystem side effects proved unreliable.

local function note(msg)
	ya.notify({ title = "probe", content = msg, timeout = 20 })
end

--- cx lives only in the sync context; plugin entry and ps.sub callbacks in the
--- async VM cannot touch it without this hop.
local read_state = ya.sync(function()
	local tab = cx.active
	local hovered = tab.current.hovered
	local marked = {}
	for _, url in pairs(tab.selected) do
		marked[#marked + 1] = tostring(url)
	end
	return {
		cwd = tostring(tab.current.cwd),
		hovered = hovered and tostring(hovered.url) or "",
		hovered_is_dir = hovered and hovered.cha.is_dir or false,
		marked = marked,
		marked_n = #marked,
		tab_idx = cx.tabs.idx,
		tab_count = #cx.tabs,
	}
end)

local M = {}

function M:setup()
	ps.sub("hover", function(body)
		-- body carries only `tab` locally, so the state has to be re-read.
		local s = read_state()
		ps.pub_to(0, "spike-state", s)
	end)
	ps.sub("cd", function(body)
		local s = read_state()
		ps.pub_to(0, "spike-cd", s)
	end)
end

function M:entry(job)
	local s = read_state()
	if job.args[1] == "pub" then
		ps.pub_to(0, "spike-state", s)
		note("published")
		return
	end
	note(string.format(
		"cwd=%s hov=%s dir=%s marked=%d tab=%d/%d",
		s.cwd:gsub(".*/", ""),
		s.hovered:gsub(".*/", ""),
		tostring(s.hovered_is_dir),
		s.marked_n,
		s.tab_idx,
		s.tab_count
	))
end

return M
