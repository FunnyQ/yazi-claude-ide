# yazi capability baseline

Measured 2026-08-07 against yazi 26.5.6 (Homebrew 2026-05-05) on macOS arm64.
Reproduce with `spike/yazi/harness.sh`; see `spike/yazi/README.md` for the
procedure. Where this document and the yazi docs disagree, trust this one.

## Result

**A yazi Lua plugin cannot own a long-lived child process. A sidecar is
required, and the plugin can launch it.**

## Process capability

| Method | Child runs | Child outlives `entry` |
| --- | --- | --- |
| `Command:status()` | yes | n/a — blocks until exit |
| `Command:output()` | yes | n/a — blocks until exit |
| `Command:spawn()` | **no** | no |
| `Command:spawn()`, handle held in a Lua global | **no** | no |
| `sh -c 'nohup … &'` via `:status()` (double fork) | yes | **yes** |

`spawn()` returns a non-nil child and a nil error, then the process never runs
— not even a bare `date >> file` that would finish in microseconds. Parking the
handle in a global does not help, so this is not handle-drop: the async plugin
VM is torn down when `entry` returns and takes its children with it. `spawn()`
is usable only for a child that is read to completion inside the same `entry`
call, which is exactly how the preset `json` and `zoxide` plugins use it.

A double-forked child survives **both** a normal quit and `SIGKILL` of yazi.
That is a leak, not a feature: a sidecar started this way must detect that its
yazi is gone and exit on its own.

## Reading yazi state

`cx` exists only in the sync context. Plugin `entry` and `ps.sub` callbacks run
async and must hop through `ya.sync`:

```lua
local read_state = ya.sync(function()
  local tab = cx.active
  local hovered = tab.current.hovered
  return {
    cwd = tostring(tab.current.cwd),
    hovered = hovered and tostring(hovered.url) or "",
    hovered_is_dir = hovered and hovered.cha.is_dir or false,
    tab_idx = cx.tabs.idx,
    tab_count = #cx.tabs,
  }
end)
```

All four fields the MVP needs are readable and correct: focused file, marked
files (`cx.active.selected`), current tab, and cwd.

## Getting state out of yazi

Three channels, measured:

1. **`ya sub <kinds>` from any process, no plugin at all.** Yields
   `kind,receiver,sender,json` lines. `hover` and `cd` carry
   `{"tab":N,"url":"…"}` — a path and nothing else, which is already the whole
   MVP payload.
2. **`ps.pub_to(0, "<kind>", table)` from a plugin.** An arbitrary Lua table
   reaches an external `ya sub <kind>` as JSON. This is how marked files, tab
   count, or file contents get out. `ya sub` accepts a kind nobody built in —
   `claude-marked` subscribes and delivers like `hover` does.

   **`ps` exists only in the sync context.** A plugin's `entry()` runs in the
   async VM, where `ps` is `nil`, so `ps.pub_to` there fails the whole plugin
   call — the publish has to move inside the `ya.sync` hop that reads `cx`. The
   failure is quiet in the worst way: no notification, no stderr, nothing on the
   DDS wire. The only trace is a `Run plugin '<name>'` entry in yazi's task
   manager (`w`), and the error text appears only after pressing Enter on it:

   ```
   runtime error: ?:?: attempt to index a nil value (field 'ps')
   stack traceback:
       ?: in function 'claude-ide.entry'
   ```
3. **`ya emit-to <id> <action>`** drives yazi from outside. Combined with (1)
   this makes the whole integration testable headlessly — unlike the Claude
   Code side, which has no headless path at all.

### The local `ps.sub` body is not the DDS body

Over the wire a `hover` message carries `url`. Inside a plugin, the same
`ps.sub("hover", fn)` callback receives a body exposing only `tab`; `body.url`
is `nil`. A plugin reacting to hover must re-read state through `ya.sync`.

### Events are redundant and arrive before state settles

`hover` fires repeatedly with an unchanged url, and the first events after
startup carry `hovered: ""`. Any consumer has to dedupe and tolerate empty
state.

## Instance identity

`ya sub` is **global**: one subscriber receives events from every yazi instance
on the machine. Messages are distinguished by the `sender` field, which is the
instance's `YAZI_ID` — a start timestamp in microseconds, not a pid.

`YAZI_ID` **is inherited by processes a plugin spawns**, so a plugin-launched
sidecar knows which instance it belongs to and can filter on it. A sidecar
started any other way cannot learn this.

`$PPID` inside a plugin's child is a worker process, **not** yazi. yazi's own
pid is not obtainable this way, which matters because the lock file needs one.

## Lifecycle detection is missing

`ya sub hey` delivers a full peer roster on join, including each peer's declared
message kinds:

```
hey,0,<sender>,{"peers":{"<id>":{"abilities":["spike-cd","spike-state"]},…},"version":"26.5.6 Homebrew"}
```

There is **no `bye` on exit**, and no fresh `hey` when a peer leaves — measured
by quitting an instance and watching for 10s. A sidecar therefore cannot learn
from DDS that its yazi died. It must poll.

Re-measured on 26.5.6 with `ya sub hi,hey,bye`, quitting an instance through
`ya emit-to <id> quit`: still no `bye` and no fresh `hey`. The binary does carry
a `bye` kind (`unit struct EmberBye`, and `hi hey bye hover …` in its kind list),
so the message type exists but is not delivered to subscribers. Do not plan
around it.

## Launching from `init.lua` (2026-08-08)

Measured while wiring the real plugin, and it corrects two assumptions above.

**`cx` does not exist in `setup()`.** A plugin's `setup()` runs from `init.lua`
before the app state does, so reading `cx.active.current.cwd` there aborts yazi
outright:

```
Error: Lua runtime failed
    runtime error: [string "claude-ide"]:11: attempt to index a nil value (global 'cx')
```

`ya.sync` does not help — it is the hop *into* the sync context, and there is no
state to hop into yet. A plugin that needs yazi's directory at startup must take
it from the DDS stream instead.

**The double fork works from `setup()`.** `Command("sh"):arg({"-c", "nohup … &"}):status()`
launches the sidecar during yazi's startup, with `YAZI_ID` inherited. Because
`setup()` runs exactly once per instance, this is also what keeps a single yazi
to a single sidecar — no guard needed.

**`cd` fires at startup, and carries the directory yazi actually opened.** The
first lines of a `ya sub hover,cd` stream, from `yazi ~/some/dir`:

```
cd,0,<sender>,{"tab":1,"url":"/…/sandbox"}
hover,0,<sender>,{"tab":1,"url":null}
hover,0,<sender>,{"tab":1,"url":"/…/sandbox/two.txt"}
```

This is what makes the missing `cx` harmless: the directory arrives milliseconds
later on a channel the sidecar is already reading.

**`hover` can carry `url: null`.** Not the empty string this document reported
earlier from the plugin-side view — over the wire it is JSON `null`. A consumer
must reject both.

## Liveness probe (2026-08-08)

`ya emit-to <id> <action>` **exits 0 for a live receiver and 1 for an unknown
one**, so the poll G3 needs is one exit code. Measured against a real yazi under
`dev/manual/harness.sh`, 26.5.6:

```
$ ya emit-to <live id> noop            ; echo $?
0
$ ya emit-to <unknown id> noop         ; echo $?
Cannot emit command: Receiver `<id>` not found. Check if the receiver is running.
1
```

Both cases return in **~7ms**, and 20 sequential probes take 134ms total. There
is no slow path to time out around.

`noop` is the action to probe with. `emit-to` does not validate the action name —
an unrecognised name also exits 0 — but an unrecognised name is a wasted chance
of a yazi-side error toast, and `noop` is a real command that changes nothing.

**The signal appears within 200ms of yazi's death.** Polling every 100ms after
`tmux kill-server`, the probe returned 0 at t=100ms and 1 at t=200ms. The DDS
roster does not update instantly, so a probe may report a dead yazi as alive for
one tick.

Two candidates were measured and rejected:

- **`ya sub hey` on demand.** It does work — each fresh `ya sub hey` receives a
  full roster, so a sidecar could join, look for its own `YAZI_ID` under `peers`,
  and leave. But it costs a subprocess, a JSON parse, and a timeout for the case
  where no roster arrives, to answer the same question one exit code answers.
- **Walking `ps -o ppid=` up from the sidecar.** Dead on arrival, and for a
  stronger reason than "`$PPID` is a worker": the double fork reparents the
  sidecar to `launchd` (`ppid` 1) immediately. Measured — step one of the walk
  reaches `/sbin/launchd`. Any sidecar reachable through a `ppid` chain would be
  one that dies with yazi, which is exactly what the double fork exists to
  prevent.

**`pub-to` is not a substitute.** `ya pub-to <live id> <kind>` exits 1 with
`does not have the ability to receive` unless the receiver declared that kind, so
its exit code conflates "no such peer" with "peer without the ability". Only its
stderr text separates them.

### The false-positive the poll has to survive

A probe failure means "DDS could not route to that id", which is not identical to
"yazi is gone". Server succession (below) produces exactly that: a ~1.6s window
in which every live peer is unroutable.

That window sets the threshold. At a 2s poll:

| Consecutive failures required | Outage needed to force a false exit | Margin over the measured 1.60s |
| --- | --- | --- |
| 1 | one probe inside the window | none — ~80% of server deaths |
| 2 | 2s | 25% |
| 3 | 4s | 150% |

So the sidecar requires **three**, and exits ~6s after yazi rather than ~2s.
That is the price of not dropping a live IDE every time an unrelated yazi closes.

## DDS server succession (2026-08-08)

The first yazi instance to start becomes the DDS server; later instances connect
to it as clients (`Connected to existing DDS server on instance <id>`).

**Measuring this does not require killing a real session.** The socket is
`<temp_dir>/yazi+<uid>/.dds.sock` — `$TMPDIR/yazi+501/.dds.sock` on macOS — and
both yazi and `ya` resolve it through Rust's `temp_dir()`, which reads `TMPDIR`.
A private TMPDIR therefore holds its own server election, invisible in both
directions: `ya emit-to <id>` routes inside it and exits 1 through the real DDS.
Reproduce with `spike/yazi/dds-succession.sh`.

Two traps in that method, both hit:

- **`sun_path` is capped at 104 bytes.** A TMPDIR that overruns it makes yazi
  start with DDS silently disabled — the `yazi+<uid>` directory appears, the
  socket does not, and nothing says so on screen. Keep the path short.
- **`lsof` has to be given the socket path.** Run against a yazi *process* it
  reports only unnamed unix sockets, which is why an earlier session recorded the
  socket as not locatable. `lsof <path>` names the server outright, and only the
  server appears — a client's connection to it does not.

### Succession is not instant

Killing the server leaves **every** surviving peer unroutable for **~1.5-1.6s**.
Six trials, polled at 100ms with the sidecar's own probe:

| Peers | How the server left | Longest outage | Peers permanently lost |
| --- | --- | --- | --- |
| 3 | `SIGKILL` | 1.58s | 0 |
| 3 | `quit` | 1.60s | 0 |
| 8 | `SIGKILL` | 1.54s | **1** |
| 8 | `SIGKILL` | 1.53s | 0 |
| 8 | `SIGKILL` | 1.55s | 0 |
| 8 | `SIGKILL` | 1.52s | 0 |

- **A normal `quit` is no gentler than `SIGKILL`.** There is no handoff.
- **The outage does not grow with the peer count.** Eight peers recover as fast
  as two.
- **The next-oldest instance takes over** and picks up the listening socket.
- **Once in four eight-peer trials, one live yazi never came back.** It kept
  running with its TUI intact, absent from the roster, unroutable indefinitely,
  holding no fd on the socket. Succession is a race and a peer can lose it
  outright. A sidecar attached to that instance would exit, which is the honest
  outcome: an instance off DDS can no longer deliver `hover` or `cd` either.

Killing a *client* is the control, and it is clean: only that id goes
unroutable, instantly and permanently, and no other peer records a single
failure.

### The sidecar's `ya sub` stream survives succession

A long-lived `ya sub hover,cd` keeps running across the server's death and keeps
delivering the surviving instance's events. Verified with a negative control that
kills the stream first and turns the check red. So only `emit-to` feels the
outage — a sidecar that rides it out has lost nothing.

## Command dispatch

`plugin <name> <args>` is correct in a `[[mgr.prepend_keymap]]` binding, and
`ya emit-to <id> plugin <name> <args>` works identically. `ya exec` prints an
incrementing request id, **not** the plugin's return value, so a plugin's return
value is not readable from outside.

Named args follow the documented form (`plugin foo bar --flag=x` →
`job.args[1]`, `job.args.flag`).

## Line numbers: no selection, but an exact anchor (2026-08-08)

Measured while asking whether the plugin could ever send a line range.

**yazi has no text selection.** There is no cursor inside the preview pane and no
visual mode over its contents. `cx.active.mode` reports `normal` while the user
scrolls a file — that is the *file list's* visual mode, used for marking files,
and it says nothing about text.

**`cx.active.preview.skip` is the 0-indexed top visible line, and it is exact.**
Hovering `docs/contract.md` and pressing `J` three times moved it from `0` to
`64`, with the preview's first line showing `## F. Tools that are out of scope
but still called` — that file's line 65.

Everything else on `preview` is narrower than it looks:

| Field | Result |
| --- | --- |
| `skip` | the offset, as above |
| `folder` | `nil` for a hovered file |
| `url`, `cha` | `attempt to get an unknown field` |

Two shapes of the object matter for anyone probing further. `cx.active.preview`
is **userdata, not a table**, so `pairs()` over it throws and each field has to
be read singly. And the preview's *height* is not reachable from a plugin's
`entry()` — `job.area.h` exists only inside a previewer — so even the visible
range cannot be computed. The top line is the only solid number.

So a range is constructible but would be **invented**: a plugin holding the first
`skip` and sending `start..current` on a second keypress. yazi itself offers no
such concept.

## Trap: the plugin VM's filesystem writes are unreliable

`io.open` is available in the sync VM and writes fine from `setup`. Attempting
to log from `entry` by spawning `sh -c '… >> file'` silently produces nothing,
because of the `spawn()` behaviour above. Half a session was spent concluding
"entry never runs" from a logging channel that was itself broken.

**Use `ya.notify` to observe plugin `entry`**, and read it back with
`tmux capture-pane`. It is the only channel proven to work from inside `entry`.
