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

local function visual_mode()
  local mode = vim.api.nvim_get_mode().mode
  -- Trust the LIVE mode, never `visualmode()`: that reports the last COMPLETED
  -- visual mode, so a fresh `V` right after a charwise selection would be read
  -- charwise and publish a single character instead of whole lines.
  if mode == "v" or mode == "V" or mode == "\22" then
    return mode
  end
  return nil
end

--- The selection, in the two conventions clause I4 defines: lines 1-based and
--- inclusive, characters 0-based with an exclusive end.
---
--- `char_end` is the whole reason this function exists. For a linewise
--- selection it is the length of the last selected line, and ending anywhere
--- short of that drops that line from Claude's count — a 5-to-10 selection
--- reads as 5 lines, not 6. Only the editor knows that length; the sidecar
--- would have to read the file to find it, which it never does.
local function selection(mode)
  local anchor = vim.fn.getpos("v")
  local cursor = vim.api.nvim_win_get_cursor(0)
  local first = { line = anchor[2], col = anchor[3] }
  local last = { line = cursor[1], col = cursor[2] + 1 }
  if first.line > last.line or (first.line == last.line and first.col > last.col) then
    first, last = last, first
  end

  local ok, lines = pcall(vim.fn.getregion, vim.fn.getpos("v"), vim.fn.getpos("."), { type = mode })
  if not ok or type(lines) ~= "table" or #lines == 0 then
    return nil
  end

  local char_start, char_end
  if mode == "V" then
    char_start, char_end = 0, #lines[#lines]
  else
    char_start, char_end = first.col - 1, last.col
  end

  local text = table.concat(lines, "\n")
  return {
    lineStart = first.line,
    lineEnd = last.line,
    charStart = char_start,
    charEnd = char_end,
    -- Dropped above the cap, which costs the line count but keeps the range.
    text = #text <= MAX_TEXT_BYTES and text or nil,
  }
end

--- A zero-width range at the cursor: how the editor says the selection is gone.
--- The sidecar reads same-line-same-character as empty and clears the display.
local function dismissed()
  local cursor = vim.api.nvim_win_get_cursor(0)
  return {
    lineStart = cursor[1],
    lineEnd = cursor[1],
    charStart = cursor[2],
    charEnd = cursor[2],
  }
end

local function publish(body)
  local path = vim.fn.expand("%:p")
  if path == "" then
    return
  end

  body.yaziId = vim.env.YAZI_ID
  body.url = path

  vim.system(
    { "ya", "pub-to", "0", "claude-editor-selection", "--json", vim.json.encode(body) },
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
-- Whether Claude is currently displaying a selection of ours. Without it, every
-- cursor move in normal mode would publish another dismissal — thousands of
-- them, fighting the `hover` pushes yazi itself is making.
local showing = false

local function cancel_pending()
  if timer then
    timer:stop()
    timer = nil
  end
end

vim.api.nvim_create_autocmd({ "CursorMoved", "ModeChanged" }, {
  group = vim.api.nvim_create_augroup("yazi_claude_ide_selection", { clear = true }),
  callback = function()
    local mode = visual_mode()

    if not mode then
      -- Left visual mode. Drop the queued publish first: letting it land after
      -- the dismissal would put the old selection straight back on screen.
      cancel_pending()
      if showing then
        showing = false
        publish(dismissed())
      end
      return
    end

    cancel_pending()
    timer = vim.defer_fn(function()
      timer = nil
      -- Re-checked inside the timer: the selection may be gone by now, and
      -- publishing then would leave Claude showing a range nobody is on.
      local live = visual_mode()
      if not live then
        return
      end
      local selected = selection(live)
      if selected then
        showing = true
        publish(selected)
      end
    end, DEBOUNCE_MS)
  end,
})
