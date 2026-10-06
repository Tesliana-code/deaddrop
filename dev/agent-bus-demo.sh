#!/usr/bin/env bash
# Agent Bus V0 demo: one shared room, # d34ddr0p, with the human and three
# live agents (Klodik: claude CLI, conversation only; GitHub: the existing
# read-only github-inspector worker; Research: claude CLI, web search only).
# Every node is an ordinary Deaddrop peer; the room is ordinary signed
# messages, fanned out per member. Development only.
#
#   dev/agent-bus-demo.sh up              build, reset, start relay + agents
#   dev/agent-bus-demo.sh status          each process, last log lines
#   dev/agent-bus-demo.sh tui             open the TUI in # d34ddr0p
#   dev/agent-bus-demo.sh restart-klodik  stop and start Klodik only
#   dev/agent-bus-demo.sh audit           messages the human authored (read-only)
#   dev/agent-bus-demo.sh down            stop this demo's processes only
#
# State: target/agent-bus-demo (gitignored). Relay 127.0.0.1:18807.
# Only processes recorded in this demo's pid files are ever stopped.
#
# Room membership: all four nodes trust each other (a channel) and share
# rooms.json. Each agent acts only on requests from the human (its policy).
# This script never sends a message as the human; automation has its own
# namespace and identity (dev/agent-bus-smoke.sh).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
D=${AGENT_BUS_DEMO_DIR:-$ROOT/target/agent-bus-demo}
BIN=$ROOT/target/debug
PORT=${AGENT_BUS_DEMO_PORT:-18807}
R=http://127.0.0.1:$PORT
AGENT_WIRE=${AGENT_WIRE_REPO:-/home/superadmin/projects/agent-wire-deaddrop-worker-endpoint-v0}
INSPECTOR=$ROOT/target/agent-wire/debug/github-inspector
HUMAN=${AGENT_BUS_DEMO_HUMAN:-iva}
IVA=$HUMAN:local:deaddrop
GITHUB=github:agent:deaddrop
RESEARCH=research:agent:deaddrop
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
  launch klodik env DEADDROP_HOME="$D/klodik" KLODIK_ALLOW=$IVA KLODIK_POLL_MS=500 \
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

  # Separate nodes, separate keys. Room members trust each other: trust is
  # a channel, nothing more.
  d "$HUMAN" init --node $IVA --relay "$R" >/dev/null
  for a in "${AGENTS[@]}"; do
    d "$a" init --node "$a:agent:deaddrop" --relay "$R" >/dev/null
  done
  local homes=("$HUMAN" "${AGENTS[@]}")
  local ids=("$IVA" klodik:agent:deaddrop github:agent:deaddrop research:agent:deaddrop)
  for x in 0 1 2 3; do for y in 0 1 2 3; do
    [ $x = $y ] || d "${homes[$x]}" peer add "${ids[$y]}" "$(key "${homes[$y]}")" >/dev/null
  done; done
  local members; members=$(printf '"%s",' "${ids[@]}"); members=${members%,}
  for h in "${homes[@]}"; do
    printf '{"rooms":[{"name":"d34ddr0p","members":[%s]}]}\n' "$members" > "$D/$h/rooms.json"
  done

  start_klodik
  launch github env DEADDROP_HOME="$D/github" GITHUB_PEER_ALLOW=$IVA,$RESEARCH GITHUB_PEER_POLL_MS=500 \
    GITHUB_INSPECTOR="$INSPECTOR" GITHUB_PEER_DEFAULT_REPO=Tesliana-code/deaddrop \
    "$BIN"/deaddrop-github
  launch research env DEADDROP_HOME="$D/research" RESEARCH_ALLOW=$IVA RESEARCH_POLL_MS=500 \
    RESEARCH_MAY_ASK=$GITHUB \
    "$BIN"/deaddrop-research
  sleep 1
  for p in relay "${AGENTS[@]}"; do
    alive "$p" || { echo "$p failed to start:" >&2; cat "$D/$p.log" >&2; down; exit 1; }
  done
  echo "relay $R · # d34ddr0p · klodik + github + research running"
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
  # /task::wire policy: the three workers tasks may ask (local policy only;
  # each worker still enforces its own ALLOW).
  tui) shift; DEADDROP_TASK_MAY_ASK=klodik:agent:deaddrop,$GITHUB,$RESEARCH \
    exec "$BIN"/deaddrop-tui --home "$D/$HUMAN" --open '#d34ddr0p' "$@" ;;
  restart-klodik) restart_klodik ;;
  audit) audit ;;
  down) down ;;
  *) sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
