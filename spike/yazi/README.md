# yazi-capability-spike

Answers PLAN.md task #2: can a yazi Lua plugin start and manage a long-lived
child process?

**No.** A child spawned with `Command:spawn()` never runs once `entry` returns,
even with its handle held. Only a double-forked child survives — and it survives
too well, outliving `SIGKILL` of yazi. **The architecture needs a sidecar, and
the plugin launches it.**

Full measurements: [`docs/yazi-capability.md`](../../docs/yazi-capability.md).

## Layout

| Path | What it is |
| --- | --- |
| `harness.sh` | headless driver — tmux + `ya sub` + `ya emit-to` |
| `config/` | isolated `YAZI_CONFIG_HOME`, never touches a real yazi config |
| `config/plugins/hello.yazi/` | process-capability probe (keys 1-7) |
| `config/plugins/probe.yazi/` | `cx` state reader and publisher (keys 8-9) |
| `sandbox/` | fixed file tree so hover paths are predictable |
| `fixtures/` | captured `ya sub` streams, paths normalised |

## Run it

```sh
./harness.sh start          # yazi in a private tmux server, own config
./harness.sh key 0          # canary: cursor jumps 2 rows if keymap.toml loaded
./harness.sh key 1          # Command:spawn()
./harness.sh pane           # read the ya.notify result off the screen
./harness.sh stop           # kill yazi, subscribers, and spawned workers
```

Probe keys:

| Key | Mode | Expected |
| --- | --- | --- |
| 1 | `Command:spawn()` | notify says `child=true`, **no file written** |
| 2 | `Command:status()` | file written |
| 3 | `Command:output()` | file written, stdout captured |
| 4 | long-lived `spawn()` | no file — child killed with the VM |
| 5 | long-lived `spawn()`, handle held | no file — holding does not help |
| 6 | double-forked child | file grows once per second, survives quit |
| 7 | env probe | prints `YAZI_ID`, proving children inherit it |
| 8 | read `cx` | notify shows cwd, hovered, marked count, tab |
| 9 | publish `cx` | JSON reaches an external `ya sub spike-state` |

Watch the state stream while driving yazi by hand:

```sh
./harness.sh sub spike-state,spike-cd /tmp/yazi-spike-events.log
./harness.sh key j
tail -1 /tmp/yazi-spike-events.log
```

## Reading the results

`ya.notify` is the only reporting channel that works from inside `entry`.
Do not add `io.open` or `Command`-based logging to a probe — a spawned logger
is silently dropped, which reads as "the plugin never ran" and is wrong. This
mistake cost most of the session.

`ya sub` receives events from **every** yazi on the machine. Filter by the
`sender` field or a stray real session will pollute the capture. `fixtures/`
was scrubbed for exactly this reason.

## Not tested

- What happens to remaining peers when the instance acting as DDS server exits.
  Testing it meant killing unrelated live yazi sessions.
- Plugin hot-reload. yazi has no reload action; every probe run restarts yazi.
