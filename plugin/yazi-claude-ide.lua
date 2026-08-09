-- Publishes this editor's live selection to the yazi-claude-ide sidecar, so
-- Claude sees line ranges the file manager itself has no way to know about.
-- Selecting is the whole gesture — there is no keybinding and no setup call.
--
-- This file is the reference implementation of section I of the contract. Any
-- editor can take its place by publishing the same body; see dev/docs/contract.md.
--
-- It lives in `plugin/` rather than behind a `setup()` for two reasons. It
-- depends on no plugin, so there is nothing to defer it behind. And a lazy.nvim
-- spec that carries `keys` REPLACES another spec's `keys` for the same plugin
-- rather than merging them, which is a quiet way to delete a user's bindings.
--
-- $YAZI_ID is inherited from the yazi that opened this editor, and it is also
-- what routes the message: `ya pub-to 0` is a broadcast every sidecar on the
-- machine receives, and each keeps only what carries its own id (I3).
if not vim.env.YAZI_ID then
  return
end

-- Above this the text is dropped and only the range goes out. `ggVG` is one
-- keystroke, and the whole file would otherwise cross DDS and the WebSocket to
-- drive a line count (I5).
local MAX_TEXT_BYTES = 100 * 1024

-- Long enough that dragging a selection does not spawn a process per keystroke,
-- short enough that the chip keeps up with the cursor.
local DEBOUNCE_MS = 100

local function visual()
  return vim.fn.mode():match("^[vV\22]") ~= nil
end

--- Measured: Claude counts its "N lines selected" display from the text, not
--- from the range, so a range without text draws the plain file chip instead.
--- Contents also reach the agent verbatim — see the contract before widening this.
local function selected_text()
  local ok, lines = pcall(vim.fn.getregion, vim.fn.getpos("v"), vim.fn.getpos("."), { type = vim.fn.mode() })
  if not ok or type(lines) ~= "table" then
    return nil
  end
  local text = table.concat(lines, "\n")
  return #text <= MAX_TEXT_BYTES and text or nil
end

local function publish()
  local path = vim.fn.expand("%:p")
  if path == "" then
    return
  end

  -- 1-based and inclusive, the way editors count. The sidecar owns the
  -- conversion to the 0-based pair the CLI reads (I4).
  local first, last = vim.fn.line("v"), vim.fn.line(".")
  if first > last then
    first, last = last, first
  end

  local payload = vim.json.encode({
    yaziId = vim.env.YAZI_ID,
    url = path,
    lineStart = first,
    lineEnd = last,
    text = selected_text(),
  })

  vim.system(
    { "ya", "pub-to", "0", "claude-editor-selection", "--json", payload },
    { text = true },
    function(done)
      if done.code ~= 0 then
        -- Loud on purpose. This channel's only other symptom is silence.
        vim.schedule(function()
          vim.notify("yazi-claude-ide: " .. (done.stderr or "ya pub-to failed"), vim.log.levels.ERROR)
        end)
      end
    end
  )
end

local timer

vim.api.nvim_create_autocmd({ "CursorMoved", "ModeChanged" }, {
  group = vim.api.nvim_create_augroup("yazi_claude_ide_selection", { clear = true }),
  callback = function()
    if not visual() then
      return
    end
    if timer then
      timer:stop()
    end
    timer = vim.defer_fn(function()
      timer = nil
      -- Re-checked inside the timer: the selection may be gone by now, and
      -- publishing then would leave Claude showing a range nobody is on.
      if visual() then
        publish()
      end
    end, DEBOUNCE_MS)
  end,
})
