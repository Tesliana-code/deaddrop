#!/usr/bin/env bash
# Automated smoke checks for the agent-bus demo, in their own namespace
# and with their own identity, `smoke:local:deaddrop`. Never the human's
# home or key: the human demo (dev/agent-bus-demo.sh) is for a person.
#
#   dev/agent-bus-smoke.sh prove-silent     a fresh demo, its agents
#                                              started, restarted and synced,
#                                              authors nothing as its human
#   dev/agent-bus-smoke.sh up               isolated live network
#   dev/agent-bus-smoke.sh say <agent> <text>  send as smoke:local:deaddrop
#   dev/agent-bus-smoke.sh tui              open the TUI as the smoke identity
#   dev/agent-bus-smoke.sh down             stop the isolated network
#
# State: target/agent-bus-smoke (gitignored). Relays 127.0.0.1:18809
# (live) and 127.0.0.1:18811 (prove-silent).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
SMOKE=$ROOT/target/agent-bus-smoke
HUMAN_DEMO=$ROOT/target/agent-bus-demo
DEMO=$ROOT/dev/agent-bus-demo.sh
LIVE=$SMOKE/live

# The smoke identity's namespace must never be the human demo's.
guard() {
  case "$(realpath -m "$1")" in
    "$(realpath -m "$HUMAN_DEMO")"*) echo "refusing: $1 is the human demo home" >&2; exit 1 ;;
  esac
}

demo() {
  local dir=$1 port=$2; shift 2
  guard "$dir"
  AGENT_BUS_DEMO_DIR=$dir AGENT_BUS_DEMO_PORT=$port AGENT_BUS_DEMO_HUMAN=smoke "$DEMO" "$@"
}

prove_silent() {
  local dir; dir=$SMOKE/silent-$(date +%Y%m%dT%H%M%S)
  demo "$dir" 18811 up >/dev/null
  sleep 7                       # several background sync passes by every agent
  demo "$dir" 18811 restart-klodik >/dev/null
  sleep 5
  local audit; audit=$(demo "$dir" 18811 audit)
  demo "$dir" 18811 down
  echo "$audit"
  if [ "$(echo "$audit" | head -1)" = "authored by smoke:local:deaddrop: 0" ]; then
    echo "PASS: demo start, agent restart and sync authored nothing as the human ($dir)"
  else
    echo "FAIL: the demo authored messages as its human" >&2
    exit 1
  fi
}

case ${1:-} in
  prove-silent) prove_silent ;;
  up) demo "$LIVE" 18809 up ;;
  say)
    agent=${2:?usage: say <klodik|github|research> <text>}; shift 2
    guard "$LIVE/smoke"
    DEADDROP_HOME="$LIVE/smoke" "$ROOT"/target/debug/deaddrop send "$agent:agent:deaddrop" "$*"
    ;;
  tui) demo "$LIVE" 18809 tui ;;
  status) demo "$LIVE" 18809 status ;;
  audit) demo "$LIVE" 18809 audit ;;
  down) demo "$LIVE" 18809 down ;;
  *) sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
