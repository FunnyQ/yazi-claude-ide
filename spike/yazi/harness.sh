#!/usr/bin/env bash
# Headless test harness for the yazi-capability-spike.
#
# Runs yazi in a dedicated tmux server so key injection and screen capture work
# without a human at a terminal. Uses its own tmux socket (-L yazispike) and its
# own YAZI_CONFIG_HOME so it can never disturb a real yazi session or config.
#
#   ./harness.sh start           launch the probe instance
#   ./harness.sh key 1           send a key
#   ./harness.sh pane            dump the screen (notifications land here)
#   ./harness.sh id              print this instance's YAZI_ID
#   ./harness.sh sub KINDS FILE  subscribe to DDS messages in the background
#   ./harness.sh stop            tear everything down
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SOCKET=yazispike
SESSION=s
CONFIG="$HERE/config"
SANDBOX="$HERE/sandbox"

tm() { tmux -L "$SOCKET" "$@"; }

case "${1:-}" in
start)
	tm kill-server 2>/dev/null || true
	sleep 1
	rm -f /tmp/yazi-spike-*
	tm new-session -d -s "$SESSION" -x 200 -y 50 \
		"env YAZI_CONFIG_HOME=$CONFIG yazi $SANDBOX"
	sleep 3
	echo "started, YAZI_ID=$("$0" id)"
	;;
key)
	shift
	tm send-keys -t "$SESSION" "$@"
	sleep 1.2
	;;
pane)
	tm capture-pane -p -t "$SESSION"
	;;
id)
	# YAZI_ID is a start timestamp, not a pid, so it cannot be derived from ps.
	# Key 7 runs the env probe, which prints it into a notification.
	tm send-keys -t "$SESSION" 7
	sleep 1.2
	tm capture-pane -p -t "$SESSION" | grep -o 'YAZI_ID=[0-9]*' | head -1 | cut -d= -f2
	;;
sub)
	shift
	kinds="$1"
	out="$2"
	: >"$out"
	nohup ya sub "$kinds" >"$out" 2>&1 &
	disown
	echo "subscribed to $kinds -> $out"
	;;
stop)
	tm kill-server 2>/dev/null || true
	pkill -f 'ya sub' 2>/dev/null || true
	# Only kills workers this harness created; matches on the spike log paths.
	pkill -f 'yazi-spike-.*\.log' 2>/dev/null || true
	rm -f /tmp/yazi-spike-*
	echo "stopped"
	;;
*)
	sed -n '2,14p' "$0"
	exit 1
	;;
esac
