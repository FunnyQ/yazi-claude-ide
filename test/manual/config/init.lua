-- YCI_COMMAND lets the harness point the plugin at a working copy without
-- editing this file; an installed build would just be on PATH.
require("claude-ide"):setup({ command = os.getenv("YCI_COMMAND") })
