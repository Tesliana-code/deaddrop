#!/usr/bin/env bash
# Chat Quality V1 demo: the three live agents (Klodik with bounded
# conversational context, GitHub, Research) and Iva in the TUI.
# Development only.
#
#   dev/chat-quality-demo.sh up              build, reset, start relay + agents
#   dev/chat-quality-demo.sh status          each process, last log lines
#   dev/chat-quality-demo.sh tui             open the TUI as iva:local:deaddrop
#   dev/chat-quality-demo.sh tui-klodik      the same, straight into Klodik
#   dev/chat-quality-demo.sh restart-klodik  stop and start Klodik only
#   dev/chat-quality-demo.sh transcript      show Klodik's kept context
#   dev/chat-quality-demo.sh audit           list messages the human identity
#                                            authored (read-only)
#   dev/chat-quality-demo.sh down            stop this demo's processes only
#
# State: target/chat-quality-demo (gitignored). Relay 127.0.0.1:18801.
# Only processes recorded in this demo's pid files are ever stopped.
#
# This script never sends a message as the human. Only a person typing in
# the TUI does. Automation must not use this home or its key; it has its
# own namespace and identity (dev/chat-quality-smoke.sh). The overrides
# below exist only so the smoke harness can prove this script stays silent
# in a namespace of its own.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
D=${CHAT_QUALITY_DEMO_DIR:-$ROOT/target/chat-quality-demo}
BIN=$ROOT/target/debug
PORT=${CHAT_QUALITY_DEMO_PORT:-18801}
R=http://127.0.0.1:$PORT
AGENT_WIRE=${AGENT_WIRE_REPO:-/home/superadmin/projects/agent-wire-deaddrop-worker-endpoint-v0}
INSPECTOR=$ROOT/target/agent-wire/debug/github-inspector
HUMAN=${CHAT_QUALITY_DEMO_HUMAN:-iva}
IVA=$HUMAN:local:deaddrop
AGENTS=(klodik github research)

d() { local who=$1; shift; DEADDROP_HOME="$D/$who" "$BIN"/deaddrop "$@"; }
key() { d "$1" identity | python3 -c 'import json,sys;print(json.load(sys.stdin)["key"])'; }
alive() { [ -f "$D/$1.pid" ] && kill -0 "$(cat "$D/$1.pid")" 2>/dev/null; }

stop() {
  if alive "$1"; then kill "$(cat "$D/$1.pid")"; fi
  rm -f "$D/$1.pid"
}

down() {
  for p in "${AGENTS[@]}" relay; do stop "$p"; done
}

# Start a process detached; record its own pid.
launch() {
  local name=$1; shift
  setsid "$@" >"$D/$name.log" 2>&1 < /dev/null &
  echo $! >"$D/$name.pid"
}

start_klodik() {
  launch klodik env DEADDROP_HOME="$D/klodik" KLODIK_ALLOW=$IVA KLODIK_POLL_SECS=2 \
    "$BIN"/deaddrop-klodik
}

restart_klodik() {
  stop klodik
  start_klodik
  sleep 0.5
  alive klodik || { echo "klodik failed to restart:" >&2; cat "$D/klodik.log" >&2; exit 1; }
  echo "klodik restarted (pid $(cat "$D/klodik.pid"))"
}

up() {
  (cd "$ROOT" && cargo build -q -p deaddrop-node -p deaddrop-cli -p deaddrop-tui \
    -p deaddrop-klodik -p deaddrop-github -p deaddrop-research)
  # The existing worker, built from its own repo into this worktree's target.
  (cd "$AGENT_WIRE" && cargo build -q --offline --locked -p github-inspector --target-dir "$ROOT/target/agent-wire")
  command -v claude >/dev/null || { echo "claude CLI not found" >&2; exit 1; }
  command -v gh >/dev/null || { echo "gh CLI not found" >&2; exit 1; }
  if [ -d "$D" ]; then down; fi
  rm -rf "$D"; mkdir -p "$D"

  launch relay "$BIN"/deaddrop-node --db "$D/relay.sqlite3" --artifacts "$D/relay-art" \
    --listen "127.0.0.1:$PORT"
  for _ in $(seq 50); do curl -s -o /dev/null "$R" && break; sleep 0.1; done

  # Separate nodes, separate keys, explicit mutual trust with Iva only.
  d "$HUMAN" init --node $IVA --relay "$R" >/dev/null
  for a in "${AGENTS[@]}"; do
    d "$a" init --node "$a:agent:deaddrop" --relay "$R" >/dev/null
    d "$HUMAN" peer add "$a:agent:deaddrop" "$(key "$a")" >/dev/null
    d "$a" peer add $IVA "$(key "$HUMAN")" >/dev/null
  done

  start_klodik
  launch github env DEADDROP_HOME="$D/github" GITHUB_PEER_ALLOW=$IVA GITHUB_PEER_POLL_SECS=2 \
    GITHUB_INSPECTOR="$INSPECTOR" GITHUB_PEER_DEFAULT_REPO=Tesliana-code/deaddrop \
    "$BIN"/deaddrop-github
  launch research env DEADDROP_HOME="$D/research" RESEARCH_ALLOW=$IVA RESEARCH_POLL_SECS=2 \
    "$BIN"/deaddrop-research
  sleep 1
  for p in relay "${AGENTS[@]}"; do
    alive "$p" || { echo "$p failed to start:" >&2; cat "$D/$p.log" >&2; down; exit 1; }
  done
  echo "relay $R · klodik + github + research running"
}

# Messages the human identity authored, from its own store, read-only (no
# sync, no writes), in local insertion order.
audit() {
  python3 - "$D/$HUMAN/messages.sqlite3" "$IVA" <<'PY'
import sqlite3, sys
path, me = sys.argv[1], sys.argv[2]
try:
    db = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    rows = db.execute(
        "select rowid, message_id, recipient, body from messages "
        "where sender = ? and kind != 'acknowledgment' order by rowid", (me,)
    ).fetchall()
except sqlite3.Error:
    rows = []
print(f"authored by {me}: {len(rows)}")
for rowid, mid, to, body in rows:
    print(f"  #{rowid} {mid} -> {to}: {body[:70]!r}")
PY
}

status() {
  for p in relay "${AGENTS[@]}"; do
    if alive "$p"; then echo "== $p: running (pid $(cat "$D/$p.pid"))"; else echo "== $p: stopped"; fi
    [ -f "$D/$p.log" ] && tail -n 4 "$D/$p.log" | sed 's/^/   /'
  done
}

case ${1:-} in
  up) up ;;
  status) status ;;
  tui) shift; exec "$BIN"/deaddrop-tui --home "$D/$HUMAN" "$@" ;;
  tui-klodik) shift; exec "$BIN"/deaddrop-tui --home "$D/$HUMAN" --open klodik:agent:deaddrop "$@" ;;
  audit) audit ;;
  restart-klodik) restart_klodik ;;
  transcript) python3 -m json.tool "$D/klodik/klodik/transcripts.json" 2>/dev/null || echo "(no completed exchanges yet)" ;;
  down) down ;;
  *) sed -n '2,23p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
