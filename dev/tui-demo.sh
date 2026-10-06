#!/usr/bin/env bash
# Local demo network for the Deaddrop TUI. Development only; not shipped.
#
#   dev/tui-demo.sh up            build, reset, start a loopback relay, seed
#   dev/tui-demo.sh tui [--ascii] open the TUI as iva
#   dev/tui-demo.sh ack <who>     <who> (danil|tanish|klodik) syncs and ACKs
#                                 everything iva sent them
#   dev/tui-demo.sh down          stop the relay
#
# State lives in target/deaddrop-demo (gitignored). Nodes: iva, danil,
# tanish, klodik, all trusting each other, on a relay at 127.0.0.1:18789.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
D=$ROOT/target/deaddrop-demo
BIN=$ROOT/target/debug
PORT=${DEADDROP_DEMO_PORT:-18789}
R=http://127.0.0.1:$PORT
NODES=(iva danil tanish klodik)

d() { local who=$1; shift; DEADDROP_HOME="$D/nodes/$who" "$BIN"/deaddrop "$@"; }
field() { python3 -c "import json,sys;print(json.load(sys.stdin)[\"$1\"])"; }

relay_running() { [ -f "$D/relay.pid" ] && kill -0 "$(cat "$D/relay.pid")" 2>/dev/null; }

down() {
  if relay_running; then kill "$(cat "$D/relay.pid")"; fi
  rm -f "$D/relay.pid"
}

up() {
  (cd "$ROOT" && cargo build -q -p deaddrop-node -p deaddrop-cli -p deaddrop-tui)
  down
  rm -rf "$D"; mkdir -p "$D/nodes"
  setsid "$BIN"/deaddrop-node --db "$D/relay.sqlite3" --artifacts "$D/relay-art" \
    --listen "127.0.0.1:$PORT" >"$D/relay.log" 2>&1 < /dev/null &
  echo $! >"$D/relay.pid"
  for _ in $(seq 50); do
    curl -s -o /dev/null "$R" 2>/dev/null && break
    sleep 0.1
  done

  for n in "${NODES[@]}"; do d "$n" init --node "$n:local:deaddrop" --relay "$R" >/dev/null; done
  for a in "${NODES[@]}"; do for b in "${NODES[@]}"; do
    [ "$a" = "$b" ] || d "$a" peer add "$b:local:deaddrop" "$(d "$b" identity | field key)" >/dev/null
  done; done

  d danil send iva:local:deaddrop "Found the auth boundary. The relay never verifies; every node does it for itself." >/dev/null
  d danil send iva:local:deaddrop "Want me to write it up?" >/dev/null
  d iva inbox >/dev/null
  local m; m=$(d iva send danil:local:deaddrop "Nice. Fix it, then write it up." | field message_id)
  d danil inbox >/dev/null; d danil ack "$m" >/dev/null
  printf 'draft notes\n' >"$D/notes.txt"
  local art; art=$(d tanish artifact put "$D/notes.txt" | field artifact_ref)
  d tanish send iva:local:deaddrop "Draft attached — look when you can." --correlation review-42 --artifact "$art" >/dev/null
  d iva send tanish:local:deaddrop "On it." >/dev/null
  d klodik send iva:local:deaddrop "ping from klodik" >/dev/null
  d iva inbox >/dev/null
  echo "relay $R (pid $(cat "$D/relay.pid")), nodes in $D/nodes"
}

ack() {
  local who=${1:?usage: tui-demo.sh ack <danil|tanish|klodik>}
  local ids
  ids=$(d "$who" inbox | python3 -c '
import json, sys
for m in json.load(sys.stdin)["messages"]:
    if m["from"] == "iva:local:deaddrop" and m["kind"] == "message":
        print(m["id"])')
  for id in $ids; do d "$who" ack "$id" >/dev/null; echo "$who acked $id"; done
}

case ${1:-} in
  up) up ;;
  tui) shift; relay_running || echo "relay is not running (dev/tui-demo.sh up); showing local state" >&2
       exec "$BIN"/deaddrop-tui --home "$D/nodes/iva" "$@" ;;
  ack) shift; ack "$@" ;;
  down) down ;;
  *) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
