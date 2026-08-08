#!/usr/bin/env bash
# Drives a real yazi with the plugin loaded, headlessly.
#
# The yazi half of the integration is scriptable; the Claude Code half is not
# (`--ide` is interactive-only, see docs/baseline.md). So this harness proves
# everything up to the lock file and the push, and a human does the last step.
#
#   ./harness.sh start           launch yazi with the plugin
#   ./harness.sh key <keys>      send keys (j/k to move, l to enter a dir)
#   ./harness.sh pane            dump the screen
#   ./harness.sh lock            print this instance's lock file
#   ./harness.sh log             print the sidecar log
#   ./harness.sh sidecar         print the sidecar process, if any
#   ./harness.sh stop            quit yazi and kill the sidecar
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
SOCKET=yaziclaudeide
SESSION=s
SANDBOX="$REPO/spike/yazi/sandbox"

# Its own config dir so a real yazi session can never be affected, and its own
# CLAUDE_CONFIG_DIR so lock files never land in the user's ~/.claude/ide.
export YAZI_CONFIG_HOME="$HERE/config"
export CLAUDE_CONFIG_DIR="$HERE/run"
export YCI_COMMAND="bun $REPO/src/sidecar.ts"

tm() { tmux -L "$SOCKET" "$@"; }

case "${1:-}" in
start)
	tm kill-server 2>/dev/null || true
	mkdir -p "$CLAUDE_CONFIG_DIR/ide"
	rm -f "$CLAUDE_CONFIG_DIR"/ide/*.lock /tmp/yazi-claude-ide-*.log
	tm new-session -d -s "$SESSION" -x 200 -y 50 \
		"env YAZI_CONFIG_HOME=$YAZI_CONFIG_HOME CLAUDE_CONFIG_DIR=$CLAUDE_CONFIG_DIR YCI_COMMAND='$YCI_COMMAND' yazi $SANDBOX"
	sleep 4
	echo "started"
	;;
key)
	shift
	tm send-keys -t "$SESSION" "$@"
	sleep 1.2
	;;
pane)
	tm capture-pane -p -t "$SESSION"
	;;
lock)
	cat "$CLAUDE_CONFIG_DIR"/ide/*.lock 2>/dev/null || echo "no lock file"
	echo
	;;
log)
	cat /tmp/yazi-claude-ide-*.log 2>/dev/null || echo "no sidecar log"
	;;
sidecar)
	pgrep -fl 'src/sidecar.ts' || echo "no sidecar running"
	;;
stop)
	tm kill-server 2>/dev/null || true
	pkill -f 'src/sidecar.ts' 2>/dev/null || true
	pkill -f 'ya sub hover,cd' 2>/dev/null || true
	echo "stopped"
	;;
*)
	sed -n '2,14p' "$0"
	exit 1
	;;
esac
