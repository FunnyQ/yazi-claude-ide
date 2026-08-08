# Manual verification harness

Drives a real yazi with the plugin loaded, in a dedicated tmux server. Uses its
own `YAZI_CONFIG_HOME` and its own `CLAUDE_CONFIG_DIR`, so it can disturb neither
a live yazi session nor the lock files in `~/.claude/ide`.

The yazi half of the integration is scriptable this way. The Claude Code half is
not — `--ide` is interactive-only, see [../../docs/baseline.md](../../docs/baseline.md).

## Run it

```sh
test/manual/harness.sh start
test/manual/harness.sh sidecar   # the sidecar the plugin launched
test/manual/harness.sh lock      # the lock file it wrote
test/manual/harness.sh log       # its stderr
test/manual/harness.sh stop
```

`verify` runs the assertions instead of printing state, and exits non-zero on any
failure. It takes about two minutes, because each case waits out the liveness
poll.

```sh
test/manual/harness.sh verify
```

It covers the clauses the unit suite cannot reach, because the unit suite injects
the very things under test — the liveness probe and the pid check:

| Case | Clause | Assertion |
| --- | --- | --- |
| `quit` | G3, A6 | after `ya emit-to <id> quit`, the sidecar exits and removes its lock file |
| `kill` | G3, A6 | the same after `kill -9` of yazi |
| `stale` | A7 | a `kill -9`ed sidecar leaves a lock file, and the next startup reclaims it |
| `g4` | G4 | two instances get distinct ports, tokens, and sidecars, and one exiting leaves the other's lock file byte-identical |

Drive yazi with `ya emit-to` rather than keystrokes. Keys depend on where the
cursor happens to be; `emit-to` does not.

```sh
ID=$(grep -o 'yazi=[0-9]*' /tmp/yazi-claude-ide-*.log | head -1 | cut -d= -f2)
SB=$PWD/spike/yazi/sandbox

ya emit-to $ID cd "$SB/dir-a"        # cursor entry follows, anchor does not
ya emit-to $ID reveal "$SB/two.txt"  # hover, and so a selection_changed push
```

## What this proved (2026-08-08)

- The plugin's `setup()` double-forks the sidecar during yazi startup, and it
  inherits `YAZI_ID`.
- The lock file appears with the repository root as the anchor and yazi's
  directory as the cursor.
- `cd` rewrites the cursor entry alone; the anchor stays put across four
  navigations, including out of the repository's own subtree.
- `reveal` produces a `selection_changed` push carrying that file's path.
- Directories never produce a push. Only regular files do.

## What `verify` proved (2026-08-08)

The sidecar used to outlive yazi, and its lock file with it. It now polls
`ya emit-to <id> noop` and exits on three consecutive failures — see the
liveness-probe measurements in [../../docs/yazi-capability.md](../../docs/yazi-capability.md).
All four cases pass, and disabling the poll turns three of them red, so the
harness observes the behaviour rather than merely reporting green.

Exit takes about six seconds: 2s per poll, three failures.
