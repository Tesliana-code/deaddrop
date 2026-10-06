#!/usr/bin/env bash
# Three live agents: Iva in the TUI; Klodik (claude CLI, no tools), GitHub
# (the existing Agent Wire github-inspector worker, read-only) and Research
# (claude CLI, web search only) as ordinary Deaddrop peers. Development only.
#
#   dev/three-agents-live-demo.sh up       build, reset, start relay + agents
#   dev/three-agents-live-demo.sh status   each process, last log lines
#   dev/three-agents-live-demo.sh tui      open the TUI as iva:local:deaddrop
#   dev/three-agents-live-demo.sh down     stop this demo's processes only
#
# State: target/three-agents-live-demo (gitignored). Relay 127.0.0.1:18797.
# Only processes recorded in this demo's pid files are ever stopped.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
D=$ROOT/target/three-agents-live-demo
BIN=$ROOT/target/debug
PORT=18797
R=http://127.0.0.1:$PORT
AGENT_WIRE=${AGENT_WIRE_REPO:-/home/superadmin/projects/agent-wire-deaddrop-worker-endpoint-v0}
INSPECTOR=$ROOT/target/agent-wire/debug/github-inspector
IVA=iva:local:deaddrop
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
  d iva init --node $IVA --relay "$R" >/dev/null
  for a in "${AGENTS[@]}"; do
    d "$a" init --node "$a:agent:deaddrop" --relay "$R" >/dev/null
    d iva peer add "$a:agent:deaddrop" "$(key "$a")" >/dev/null
    d "$a" peer add $IVA "$(key iva)" >/dev/null
  done

  launch klodik env DEADDROP_HOME="$D/klodik" KLODIK_ALLOW=$IVA KLODIK_POLL_SECS=2 \
    "$BIN"/deaddrop-klodik
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

status() {
  for p in relay "${AGENTS[@]}"; do
    if alive "$p"; then echo "== $p: running (pid $(cat "$D/$p.pid"))"; else echo "== $p: stopped"; fi
    [ -f "$D/$p.log" ] && tail -n 4 "$D/$p.log" | sed 's/^/   /'
  done
}

case ${1:-} in
  up) up ;;
  status) status ;;
  tui) shift; exec "$BIN"/deaddrop-tui --home "$D/iva" "$@" ;;
  down) down ;;
  *) sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
