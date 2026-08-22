# Manual verification harness

Drives a real yazi with the plugin loaded, in a dedicated tmux server. Uses its
own `YAZI_CONFIG_HOME` and its own `CLAUDE_CONFIG_DIR`, so it can disturb neither
a live yazi session nor the lock files in `~/.claude/ide`.

The yazi half of the integration is scriptable this way. The Claude Code half is
not — `--ide` is interactive-only, see [../docs/baseline.md](../docs/baseline.md).

## Run it

```sh
dev/manual/harness.sh start
dev/manual/harness.sh sidecar   # the sidecar the plugin launched
dev/manual/harness.sh lock      # the lock file it wrote
dev/manual/harness.sh log       # its stderr
dev/manual/harness.sh stop
```

`verify` runs the assertions instead of printing state, and exits non-zero on any
failure. It takes about two minutes, because each case waits out the liveness
poll.

```sh
dev/manual/harness.sh verify
```

It covers the clauses the unit suite cannot reach, because the unit suite injects
the very things under test — the liveness probe and the pid check:

| Case | Clause | Assertion |
| --- | --- | --- |
| `quit` | G3, A6 | after `ya emit-to <id> quit`, the sidecar exits and removes its lock file |
| `kill` | G3, A6 | the same after `kill -9` of yazi |
| `stale` | A7 | a `kill -9`ed sidecar leaves a lock file, and the next startup reclaims it |
| `g4` | G4 | two instances get distinct ports, tokens, and sidecars, and one exiting leaves the other's lock file byte-identical |
| `h` | H1-H3 | marking two files and pressing `cv` publishes `claude-marked`, and the sidecar logs the set |

The `h` case is the one that needs keystrokes: `cx.active.selected` is unreadable
from outside yazi, so `ya emit-to` cannot build a marked set. Everything else
uses `emit-to`, because keys depend on where the cursor happens to be.

```sh
ID=$(grep -o 'yazi=[0-9]*' /tmp/yazi-claude-ide/logs/*.log | head -1 | cut -d= -f2)
SB=$PWD/dev/spike/yazi/sandbox

ya emit-to $ID cd "$SB/dir-a"        # cursor entry follows, anchor does not
ya emit-to $ID reveal "$SB/two.txt"  # hover, and so a selection_changed push
```

## The diff viewer (section J)

`verify` does not cover section J: the openDiff has to come from a WebSocket
client, and this harness is bash. Drive it by hand instead, with
[`../spike/diff-client.ts`](../spike/diff-client.ts) standing in for Claude.

```sh
export YCI_DIFF_CMD='nvim -d "$1" "$2"'   # launch forwards it; unset, J never runs
dev/manual/harness.sh start
bun dev/spike/diff-client.ts dev/manual/run/ide "$PWD/dev/spike/yazi/sandbox/one.txt" &
dev/manual/harness.sh pane                # nvim, holding yazi's terminal
dev/manual/harness.sh key C-w l
dev/manual/harness.sh key ':2s/TWO/AMENDED/' Enter
dev/manual/harness.sh key ':wqa' Enter
```

The client prints the reply. `FILE_SAVED` carrying the amended line is the whole
of J1-J5: the viewer got the terminal, the user's edit reached the copy, and the
publish released the held request.

### What this proved (2026-08-20)

- `ya emit-to <id> shell '<cmd>' --block` hands a real terminal to a real nvim,
  with yazi hidden behind it, when the caller is a double-forked sidecar that has
  no terminal of its own.
- The amendment survives the round trip: `one\nTWO\nthree` went out and
  `one\nAMENDED-BY-THE-USER\nthree` came back, 21 seconds later, with no timeout
  and no second request.
- The copy and its directory are gone afterwards, and the user's own file is
  untouched.
- The sidecar log carries the path and the tab name and neither side's contents
  (J8).
- **`ya pub-to` needs `--json`.** The body is an option, not a positional
  argument, and a script that omits the flag publishes nothing at all. No unit
  test can see this — the argv builder was green while the channel was dead.

## What this proved (2026-08-08)

- The plugin's `setup()` double-forks the sidecar during yazi startup, and it
  inherits `YAZI_ID`.
- The lock file appears with the repository root as the anchor and yazi's
  directory as the cursor.
- `cd` rewrites the cursor entry alone; the anchor stays put across four
  navigations, including out of the repository's own subtree.
- `reveal` produces a `selection_changed` push carrying that file's path.
- Directories never produce a push. Only regular files do.
- Pressing `cv` with two files marked publishes `claude-marked` carrying both
  absolute paths, and the sidecar logs `marked 2 file(s)`. Getting there cost a
  silent failure worth remembering: `ps` is `nil` in the async VM `entry()` runs
  in, and the only trace is a failed task under `w` — see
  [../docs/yazi-capability.md](../docs/yazi-capability.md).

## What `verify` proved (2026-08-08)

The sidecar used to outlive yazi, and its lock file with it. It now polls
`ya emit-to <id> noop` and exits on three consecutive failures — see the
liveness-probe measurements in [../docs/yazi-capability.md](../docs/yazi-capability.md).
All four cases pass, and disabling the poll turns three of them red, so the
harness observes the behaviour rather than merely reporting green.

Exit takes about six seconds: 2s per poll, three failures.
