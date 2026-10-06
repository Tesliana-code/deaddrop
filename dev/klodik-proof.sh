#!/usr/bin/env bash
# Local proof: Iva and Klodik as separate Deaddrop nodes on a loopback relay,
# Klodik answering through the local `claude` CLI (no tools). Development
# only; not shipped.
#
#   dev/klodik-proof.sh up              build, reset, start relay, create and
#                                       mutually trust iva + klodik
#   dev/klodik-proof.sh say <text>      Iva sends <text> to Klodik
#   dev/klodik-proof.sh klodik [--once] run the Klodik adapter
#   dev/klodik-proof.sh inbox           Iva syncs and shows her inbox
#   dev/klodik-proof.sh tui             open the TUI as Iva
#   dev/klodik-proof.sh down            stop the relay
#
# State: target/klodik-proof (gitignored). Relay 127.0.0.1:18790.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
D=$ROOT/target/klodik-proof
BIN=$ROOT/target/debug
PORT=${KLODIK_PROOF_PORT:-18790}
R=http://127.0.0.1:$PORT
IVA=iva:local:deaddrop
KLODIK=klodik:agent:deaddrop

d() { local who=$1; shift; DEADDROP_HOME="$D/$who" "$BIN"/deaddrop "$@"; }
key() { d "$1" identity | python3 -c 'import json,sys;print(json.load(sys.stdin)["key"])'; }
relay_running() { [ -f "$D/relay.pid" ] && kill -0 "$(cat "$D/relay.pid")" 2>/dev/null; }

down() {
  if relay_running; then kill "$(cat "$D/relay.pid")"; fi
  rm -f "$D/relay.pid"
}

up() {
  (cd "$ROOT" && cargo build -q -p deaddrop-node -p deaddrop-cli -p deaddrop-tui -p deaddrop-klodik)
  down
  rm -rf "$D"; mkdir -p "$D"
  setsid "$BIN"/deaddrop-node --db "$D/relay.sqlite3" --artifacts "$D/relay-art" \
    --listen "127.0.0.1:$PORT" >"$D/relay.log" 2>&1 < /dev/null &
  echo $! >"$D/relay.pid"
  for _ in $(seq 50); do curl -s -o /dev/null "$R" && break; sleep 0.1; done
  d iva init --node $IVA --relay "$R" >/dev/null
  d klodik init --node $KLODIK --relay "$R" >/dev/null
  # Explicit, out-of-band trust in both directions.
  d iva peer add $KLODIK "$(key klodik)" >/dev/null
  d klodik peer add $IVA "$(key iva)" >/dev/null
  echo "relay $R; iva=$D/iva klodik=$D/klodik"
}

case ${1:-} in
  up) up ;;
  say) shift; d iva send $KLODIK "$*" ;;
  klodik) shift; DEADDROP_HOME="$D/klodik" KLODIK_ALLOW=$IVA exec "$BIN"/deaddrop-klodik "$@" ;;
  inbox) d iva inbox ;;
  tui) exec "$BIN"/deaddrop-tui --home "$D/iva" ;;
  down) down ;;
  *) sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
