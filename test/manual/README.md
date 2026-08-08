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

## Known gap

The sidecar outlives yazi. After `tmux kill-server`, `pgrep -f src/sidecar.ts`
still finds it and its lock file is still in place — the process is alive, so
nothing reclaims the lock either. Contract clause G3, PLAN task #6.
