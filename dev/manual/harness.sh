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
#   ./harness.sh verify          assert the contract clauses only a real yazi shows
#   ./harness.sh stop            quit yazi and kill the sidecar
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
SOCKET=yaziclaudeide
SESSION=s
SANDBOX="$REPO/dev/spike/yazi/sandbox"

# Its own config dir so a real yazi session can never be affected, and its own
# CLAUDE_CONFIG_DIR so lock files never land in the user's ~/.claude/ide.
export YAZI_CONFIG_HOME="$HERE/config"
export CLAUDE_CONFIG_DIR="$HERE/run"
export YCI_COMMAND="$REPO/target/release/yazi-claude-ide"
# Deterministic stand-in for nvim, so `Enter` gives us a block opener that stays
# up and needs no terminal (I11).
export EDITOR="$HERE/block-opener"
export YCI_BLOCK_OPENER_LOG="$HERE/run/opened"
# J1 is opt-in, so the harness passes whatever the caller exported and nothing
# when they exported nothing. `launch` names every variable it forwards, and a
# variable missing from that line simply does not reach the sidecar.
export YCI_DIFF_CMD="${YCI_DIFF_CMD:-}"

tm() { tmux -L "$SOCKET" "$@"; }

# One yazi in its own tmux session. `verify` starts a second one for G4.
launch() { # session, directory
	tm new-session -d -s "$1" -x 200 -y 50 \
		"env YAZI_CONFIG_HOME=$YAZI_CONFIG_HOME CLAUDE_CONFIG_DIR=$CLAUDE_CONFIG_DIR YCI_COMMAND='$YCI_COMMAND' EDITOR='$EDITOR' YCI_BLOCK_OPENER_LOG='$YCI_BLOCK_OPENER_LOG' YCI_DIFF_CMD='$YCI_DIFF_CMD' yazi $2"
}

teardown() {
	tm kill-server 2>/dev/null || true
	# Kill this harness's `ya sub` by parent, never by command line. Every sidecar
	# on the machine runs a byte-identical `ya sub`, so `pkill -f` on it reaches
	# into the developer's own yazi sessions — and the only symptom there is that
	# they quietly stop pushing to Claude, with the lock file still in place.
	for sidecar in $(pgrep -f '/target/release/yazi-claude-ide$'); do
		pkill -P "$sidecar" -f '^ya sub' 2>/dev/null || true
	done
	pkill -f '/target/release/yazi-claude-ide$' 2>/dev/null || true
	pkill -f "$HERE/block-opener" 2>/dev/null || true
}

# The plugin writes one log per instance, and the sidecar's first line names both
# its port and its YAZI_ID. That pair is the only instance-to-lock-file mapping.
ids() { for f in /tmp/yazi-claude-ide-*.log; do basename "$f" .log | cut -d- -f4; done; }
port_of() { sed -n 's|.*ws://127\.0\.0\.1:\([0-9]*\).*|\1|p' "/tmp/yazi-claude-ide-$1.log"; }
field_of() { sed -n "s/.*\"$2\":\"\{0,1\}\([^,\"}]*\).*/\1/p" "$1"; }

case "${1:-}" in
start)
	cargo build --release --manifest-path "$REPO/Cargo.toml"
	teardown
	mkdir -p "$CLAUDE_CONFIG_DIR/ide"
	rm -f "$CLAUDE_CONFIG_DIR"/ide/*.lock /tmp/yazi-claude-ide-*.log
	launch "$SESSION" "$SANDBOX"
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
	# The tmux command line names the sidecar too, so match how it is really run.
	pgrep -fl '/target/release/yazi-claude-ide$' || echo "no sidecar running"
	;;
verify)
	# The clauses a scripted probe cannot show: G3, G4, and A7 against a sidecar
	# that really died. The unit suite injects both the liveness probe and the
	# pid check, so only a real yazi settles these.
	fails=0
	ok() { echo "PASS [$1] $2"; }
	bad() {
		echo "FAIL [$1] $2"
		fails=$((fails + 1))
	}
	# The poll is 3 failures at 2s, so ~6s; 15s leaves room for start-up jitter.
	await_exit() { for _ in $(seq 30); do kill -0 "$1" 2>/dev/null || return 0; sleep 0.5; done; return 1; }

	for path in quit kill; do
		"$0" start >/dev/null
		lock=$(echo "$CLAUDE_CONFIG_DIR"/ide/*.lock)
		if [ ! -f "$lock" ]; then
			bad "$path" "no lock file after start"
			continue
		fi
		pid=$(field_of "$lock" pid)
		id=$(ids | head -1)
		echo "[$path] sidecar pid=$pid yazi=$id lock=$(basename "$lock")"

		if [ "$path" = quit ]; then ya emit-to "$id" quit; else pkill -9 -f "^yazi $SANDBOX\$"; fi

		if ! await_exit "$pid"; then
			bad "$path" "sidecar $pid still alive after 15s"
		elif [ -f "$lock" ]; then
			bad "$path" "sidecar exited but left $(basename "$lock")"
		else
			ok "$path" "sidecar exited and removed its lock file"
		fi
		"$0" stop >/dev/null
	done

	# A7 end to end. Until G3 landed this could not happen — the orphan sidecar
	# stayed alive, so its own pid check passed and its lock was never stale.
	"$0" start >/dev/null
	stale=$(echo "$CLAUDE_CONFIG_DIR"/ide/*.lock)
	kill -9 "$(field_of "$stale" pid)"
	teardown
	sleep 1
	if [ ! -f "$stale" ]; then
		bad stale "killing the sidecar removed its own lock file — nothing left to reclaim"
	else
		"$0" start >/dev/null
		if [ -f "$stale" ]; then
			bad stale "startup did not reclaim $(basename "$stale")"
		else
			ok stale "startup reclaimed the killed sidecar's lock file"
		fi
	fi
	"$0" stop >/dev/null

	# G4. Two instances in different directories, so the one to be killed is
	# addressable by its own command line. `start` clears the logs, so the id
	# present before the second launch is the victim's.
	"$0" start >/dev/null
	victim_id=$(ids)
	launch t "$SANDBOX/dir-a"
	sleep 4
	survivor_id=$(ids | grep -v "^$victim_id\$" || true)
	victim="$CLAUDE_CONFIG_DIR/ide/$(port_of "$victim_id").lock"
	survivor="$CLAUDE_CONFIG_DIR/ide/$(port_of "$survivor_id").lock"
	echo "[g4] victim $victim_id -> $(basename "$victim"), survivor $survivor_id -> $(basename "$survivor")"

	if [ -z "$survivor_id" ] || [ ! -f "$victim" ] || [ ! -f "$survivor" ]; then
		bad g4 "the two instances did not get distinct lock files"
	elif [ "$(field_of "$victim" authToken)" = "$(field_of "$survivor" authToken)" ]; then
		bad g4 "the two instances share an authToken"
	elif [ "$(field_of "$victim" pid)" = "$(field_of "$survivor" pid)" ]; then
		bad g4 "the two instances share a sidecar"
	else
		ok g4 "distinct ports, tokens, and sidecars"
		before=$(cat "$survivor")
		pkill -9 -f "^yazi $SANDBOX\$"
		await_exit "$(field_of "$victim" pid)" || bad g4 "the victim's sidecar outlived its yazi"
		if [ ! -f "$survivor" ]; then
			bad g4 "the surviving instance lost its lock file"
		elif [ "$(cat "$survivor")" != "$before" ]; then
			bad g4 "the surviving instance's lock file was rewritten"
		else
			ok g4 "the survivor is untouched by the other sidecar's exit"
		fi
	fi
	"$0" stop >/dev/null

	# H1-H3. The gesture only exists inside a real yazi: `cx.active.selected` is
	# unreadable from outside, and `ps` is nil in the async VM the plugin's
	# entry() runs in, so publishing has to happen inside the ya.sync hop. A unit
	# test cannot see either half.
	"$0" start >/dev/null
	# j j lands on one.txt; space marks and steps, so two files end up marked.
	for k in j j Space Space c v; do tm send-keys -t "$SESSION" "$k"; sleep 0.5; done
	sleep 1
	if grep -q 'marked 2 file(s)' /tmp/yazi-claude-ide-*.log; then
		ok h "the marked set reached the sidecar"
	else
		bad h "pressing cv published nothing the sidecar saw"
	fi
	"$0" stop >/dev/null

	# I2, I3, I11. None of this is reachable from the Rust suite: the channel only
	# means anything with a real block opener holding the terminal, and the sender
	# question only arises because `ya pub-to` is a separate process with an id of
	# its own. The publish is a broadcast, so it also reaches every other sidecar
	# on this machine — the wrong-id case below is what keeps it from landing
	# there, and it is checked with a live one, not a mock.
	"$0" start >/dev/null
	rm -f "$YCI_BLOCK_OPENER_LOG"
	id=$(ids | head -1)
	for k in j j; do tm send-keys -t "$SESSION" "$k"; sleep 0.5; done
	tm send-keys -t "$SESSION" Enter
	sleep 2
	relay() { # yazi id
		ya pub-to 0 claude-editor-selection --json \
			"{\"yaziId\":\"$1\",\"url\":\"$SANDBOX/one.txt\",\"lineStart\":10,\"lineEnd\":20}"
		sleep 1
	}
	if [ ! -s "$YCI_BLOCK_OPENER_LOG" ]; then
		bad i "Enter did not hand the terminal to a block opener"
	else
		ok i "block opener holds the terminal: $(cat "$YCI_BLOCK_OPENER_LOG")"
		relay "not-$id"
		if grep -q 'selection ' /tmp/yazi-claude-ide-*.log; then
			bad i "a selection addressed to another yazi was acted on"
		else
			ok i "a selection addressed to another yazi is ignored"
		fi
		relay "$id"
		if grep -q "selection $SANDBOX/one.txt L10-20" /tmp/yazi-claude-ide-*.log; then
			ok i "a selection published from outside reached the sidecar past a block opener"
		else
			bad i "the sidecar saw no selection while the block opener was up"
		fi
	fi
	"$0" stop >/dev/null

	[ "$fails" -eq 0 ] || exit 1
	;;
stop)
	teardown
	echo "stopped"
	;;
*)
	sed -n '2,15p' "$0"
	exit 1
	;;
esac
