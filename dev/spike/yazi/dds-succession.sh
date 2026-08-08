#!/usr/bin/env bash
# PLAN.md gap #7: what happens to the surviving peers when the yazi instance
# acting as DDS server exits?
#
# The question went unmeasured twice because every yazi on the machine shares
# one DDS, so answering it meant killing a real session. It does not: the socket
# is `<temp_dir>/yazi+<uid>/.dds.sock`, Rust's `temp_dir()` reads `TMPDIR`, and
# `ya` reads the same variable. A private TMPDIR therefore elects its own server,
# and probes aimed at it can never reach a real session.
#
#   ./dds-succession.sh isolation   prove the private DDS is invisible both ways
#   ./dds-succession.sh kill        SIGKILL the server, watch the survivors
#   ./dds-succession.sh quit        same, through a normal `quit`
#   ./dds-succession.sh nonserver   control: kill a client instead
#   ./dds-succession.sh scale [n]   same as `kill`, with n instances (default 8)
#   ./dds-succession.sh stream      does a live `ya sub` survive succession?
#   ./dds-succession.sh stop        kill the private tmux server
#
# The isolated path has to stay short. `sun_path` is capped at 104 bytes on
# macOS, and a TMPDIR that overruns it makes yazi start with DDS silently
# disabled — no socket, and nothing on screen to say so. That is why this lives
# under /tmp rather than beside the script.
set -euo pipefail

ISO=/tmp/yazi-dds-spike
CFG="$ISO/config"
SANDBOX="$ISO/sandbox"
SOCKET=yazidds
SOCK="$ISO/tmp/yazi+$(id -u)/.dds.sock"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

export TMPDIR="$ISO/tmp"

tm() { tmux -L "$SOCKET" "$@"; }

# tmux runs `sh -c "env … yazi"`, which execs all the way down, so the pane pid
# is yazi itself.
launch() { # session, dir
	tm new-session -d -s "$1" -x 120 -y 30 \
		"env TMPDIR=$ISO/tmp YAZI_CONFIG_HOME=$CFG yazi $2"
}
pid_of() { tm list-panes -t "$1" -F '#{pane_pid}'; }

# YAZI_ID is not in yazi's own environment — it is only injected into children,
# and an externally supplied one is ignored (measured). The roster is the way in:
# every fresh `ya sub` receives a `hey` listing the peers. The subscriber itself
# appears there with abilities ["hey"], so the real instances are the emitters.
roster() {
	local out="$ISO/hey.txt"
	ya sub hey >"$out" 2>&1 &
	local p=$!
	sleep 2
	kill "$p" 2>/dev/null || true
	python3 - "$out" <<-'PY'
		import json, sys
		body = json.loads(open(sys.argv[1]).readline().rstrip("\n").split(",", 3)[3])
		print(" ".join(sorted(k for k, v in body["peers"].items() if "dds-emit" in v["abilities"])))
	PY
}

# Only the server holds the listening socket; a client's connection does not
# show up under the path. This is the lookup an earlier session missed by
# running `lsof` on the yazi process instead of on the socket.
server_pid() { lsof -t "$SOCK" 2>/dev/null | sort -u | head -1; }

setup() {
	mkdir -p "$CFG" "$SANDBOX/a" "$SANDBOX/b" "$SANDBOX/c" "$ISO/tmp"
	: >"$SANDBOX/a/file-a.txt"
	: >"$SANDBOX/b/file-b.txt"
	: >"$SANDBOX/c/file-c.txt"
}

reset() {
	tm kill-server 2>/dev/null || true
	sleep 1
	rm -rf "$ISO/tmp/yazi+$(id -u)"
	setup
}

start_three() {
	reset
	launch a "$SANDBOX/a"
	sleep 3 # a is alone, so a elects itself server
	launch b "$SANDBOX/b"
	sleep 2
	launch c "$SANDBOX/c"
	sleep 3

	PID_A=$(pid_of a) PID_B=$(pid_of b) PID_C=$(pid_of c)
	IDS=$(roster)
	SRV=$(server_pid)
	echo "  panes    : a=$PID_A b=$PID_B c=$PID_C"
	echo "  roster   : $IDS"
	echo "  socket   : held by pid $SRV"
	# YAZI_ID is a startup timestamp in microseconds, so sorting the roster gives
	# launch order. lsof has to agree with that before the mapping is trusted.
	if [ "$SRV" = "$PID_A" ]; then
		echo "  server   : a, launched first — lsof agrees with launch order"
	else
		echo "  server   : pid $SRV is NOT a — the mapping assumption is broken"
		exit 1
	fi
	ID_A=$(echo "$IDS" | cut -d' ' -f1)
	ID_B=$(echo "$IDS" | cut -d' ' -f2)
	ID_C=$(echo "$IDS" | cut -d' ' -f3)
	echo "  ids      : a=$ID_A b=$ID_B c=$ID_C"
}

# Polls with the sidecar's own probe, so the spike measures the real call.
watch() { bun "$HERE/probe-watch.ts" "$@"; }

case "${1:-}" in
isolation)
	echo "== isolation both ways =="
	start_three
	if ya emit-to "$ID_B" noop >/dev/null 2>&1; then echo "  private DDS: exit 0 — routed"; else echo "  private DDS: exit 1 — NOT routed"; fi
	if env -u TMPDIR ya emit-to "$ID_B" noop >/dev/null 2>&1; then echo "  real DDS   : exit 0 — LEAKED into the real DDS"; else echo "  real DDS   : exit 1 — invisible to real sessions"; fi
	;;
kill)
	echo "== SIGKILL the server instance =="
	start_three
	echo "  killing a (pid $PID_A, the server)"
	kill -9 "$PID_A"
	watch "$ID_A" "$ID_B" "$ID_C" 25
	echo "  socket holder after: $(server_pid || echo none)"
	;;
quit)
	echo "== normal quit of the server instance =="
	start_three
	echo "  quitting a"
	ya emit-to "$ID_A" quit
	watch "$ID_A" "$ID_B" "$ID_C" 25
	echo "  socket holder after: $(server_pid || echo none)"
	;;
nonserver)
	echo "== control: SIGKILL a client instance =="
	start_three
	echo "  killing c (pid $PID_C, a client)"
	kill -9 "$PID_C"
	watch "$ID_A" "$ID_B" "$ID_C" 15
	;;
scale)
	# Does the outage grow with the peer count? Eight matches a habitual load.
	n=${2:-8}
	echo "== SIGKILL the server with $n peers =="
	reset
	for _ in $(seq "$n"); do
		launch "s$_" "$SANDBOX/a"
		sleep 1.5
	done
	sleep 2
	ids=$(roster)
	srv=$(server_pid)
	echo "  roster ($(echo "$ids" | wc -w | tr -d ' ') peers): $ids"
	echo "  killing the server, pid $srv (s1 is pid $(pid_of s1))"
	kill -9 "$srv"
	watch $ids 25
	;;
stream)
	# The probe recovering is not the whole answer. The sidecar also holds a
	# long-lived `ya sub`, and if that stream dies with the old server the
	# sidecar stays alive and deaf — worse than the false exit the failure
	# threshold guards against.
	echo "== does a live \`ya sub\` survive succession? =="
	start_three
	out="$ISO/sub.txt"
	: >"$out"
	ya sub hover,cd >"$out" 2>&1 &
	sub=$!
	sleep 2
	ya emit-to "$ID_B" cd "$SANDBOX/c"
	sleep 1
	echo "  killing a (pid $PID_A, the server)"
	kill -9 "$PID_A"
	sleep 6 # well past the measured ~1.6s outage
	echo "  ya sub pid $sub alive after succession: $(kill -0 "$sub" 2>/dev/null && echo yes || echo no)"
	# NEGATIVE=1 kills the stream on purpose, so the check is seen to go red.
	[ -n "${NEGATIVE:-}" ] && { kill "$sub" 2>/dev/null || true; echo "  (negative control: stream killed on purpose)"; }
	before=$(grep -c "$ID_B" "$out" || true)
	ya emit-to "$ID_B" cd "$SANDBOX/a"
	sleep 2
	after=$(grep -c "$ID_B" "$out" || true)
	kill "$sub" 2>/dev/null || true
	echo "  events from b: $before before the post-kill emit, $after after"
	if [ "$after" -gt "$before" ]; then
		echo "  RESULT: the stream survived — the sidecar keeps hearing its yazi"
	else
		echo "  RESULT: the stream went deaf — a surviving sidecar would stop receiving"
	fi
	;;
stop)
	tm kill-server 2>/dev/null || true
	echo stopped
	;;
*)
	sed -n '11,18p' "$0"
	exit 1
	;;
esac
