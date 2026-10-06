#!/usr/bin/env bash
# Live chat demo: Iva in the TUI, Klodik as an ordinary Deaddrop peer
# answered by the local `claude` CLI (no tools). Development only.
#
#   dev/klodik-chat-demo.sh up       build, reset, start relay + Klodik
#   dev/klodik-chat-demo.sh tui      open the TUI as iva:local:deaddrop
#   dev/klodik-chat-demo.sh status   what is running, last Klodik log lines
#   dev/klodik-chat-demo.sh down     stop this demo's relay and Klodik
#
# State: target/klodik-chat-demo (gitignored). Relay 127.0.0.1:18791.
# Only processes recorded in this demo's pid files are ever stopped.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
D=$ROOT/target/klodik-chat-demo
BIN=$ROOT/target/debug
PORT=${KLODIK_CHAT_DEMO_PORT:-18791}
R=http://127.0.0.1:$PORT
IVA=iva:local:deaddrop
KLODIK=klodik:agent:deaddrop

d() { local who=$1; shift; DEADDROP_HOME="$D/$who" "$BIN"/deaddrop "$@"; }
key() { d "$1" identity | python3 -c 'import json,sys;print(json.load(sys.stdin)["key"])'; }
alive() { [ -f "$D/$1.pid" ] && kill -0 "$(cat "$D/$1.pid")" 2>/dev/null; }

stop() {
  if alive "$1"; then kill "$(cat "$D/$1.pid")"; fi
  rm -f "$D/$1.pid"
}

down() {
  stop klodik
  stop relay
}

up() {
  (cd "$ROOT" && cargo build -q -p deaddrop-node -p deaddrop-cli -p deaddrop-tui -p deaddrop-klodik)
  command -v "${KLODIK_CLAUDE:-claude}" >/dev/null || { echo "claude CLI not found" >&2; exit 1; }
  down
  rm -rf "$D"; mkdir -p "$D"

  setsid "$BIN"/deaddrop-node --db "$D/relay.sqlite3" --artifacts "$D/relay-art" \
    --listen "127.0.0.1:$PORT" >"$D/relay.log" 2>&1 < /dev/null &
  echo $! >"$D/relay.pid"
  for _ in $(seq 50); do curl -s -o /dev/null "$R" && break; sleep 0.1; done

  # Two separate nodes, separate keys, explicit mutual trust.
  d iva init --node $IVA --relay "$R" >/dev/null
  d klodik init --node $KLODIK --relay "$R" >/dev/null
  d iva peer add $KLODIK "$(key klodik)" >/dev/null
  d klodik peer add $IVA "$(key iva)" >/dev/null

  DEADDROP_HOME="$D/klodik" KLODIK_ALLOW=$IVA KLODIK_POLL_SECS=2 \
    setsid "$BIN"/deaddrop-klodik >"$D/klodik.log" 2>&1 < /dev/null &
  echo $! >"$D/klodik.pid"
  sleep 0.5
  alive klodik || { echo "klodik failed to start:" >&2; cat "$D/klodik.log" >&2; down; exit 1; }
  echo "relay $R · klodik running (log: $D/klodik.log)"
}

status() {
  for p in relay klodik; do
    if alive $p; then echo "$p: running (pid $(cat "$D/$p.pid"))"; else echo "$p: stopped"; fi
  done
  [ -f "$D/klodik.log" ] && tail -n 5 "$D/klodik.log"
}

case ${1:-} in
  up) up ;;
  tui) shift; exec "$BIN"/deaddrop-tui --home "$D/iva" "$@" ;;
  status) status ;;
  down) down ;;
  *) sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
