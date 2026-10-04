#!/usr/bin/env bash
# Automated smoke checks for the chat-quality demo, in their own namespace
# and with their own identity, `smoke:local:deaddrop`. Never the human's
# home or key: the human demo (dev/chat-quality-demo.sh) is for a person.
#
#   dev/chat-quality-smoke.sh prove-silent     a fresh demo, its agents
#                                              started, restarted and synced,
#                                              authors nothing as its human
#   dev/chat-quality-smoke.sh up               isolated live network
#   dev/chat-quality-smoke.sh say <agent> <text>  send as smoke:local:deaddrop
#   dev/chat-quality-smoke.sh tui              open the TUI as the smoke identity
#   dev/chat-quality-smoke.sh tui-klodik       the same, straight into Klodik
#   dev/chat-quality-smoke.sh down             stop the isolated network
#
# State: target/chat-quality-smoke (gitignored). Relays 127.0.0.1:18803
# (live) and 127.0.0.1:18805 (prove-silent).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
SMOKE=$ROOT/target/chat-quality-smoke
HUMAN_DEMO=$ROOT/target/chat-quality-demo
DEMO=$ROOT/dev/chat-quality-demo.sh
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
  CHAT_QUALITY_DEMO_DIR=$dir CHAT_QUALITY_DEMO_PORT=$port CHAT_QUALITY_DEMO_HUMAN=smoke "$DEMO" "$@"
}

prove_silent() {
  local dir; dir=$SMOKE/silent-$(date +%Y%m%dT%H%M%S)
  demo "$dir" 18805 up >/dev/null
  sleep 7                       # several background sync passes by every agent
  demo "$dir" 18805 restart-klodik >/dev/null
  sleep 5
  local audit; audit=$(demo "$dir" 18805 audit)
  demo "$dir" 18805 down
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
  up) demo "$LIVE" 18803 up ;;
  say)
    agent=${2:?usage: say <klodik|github|research> <text>}; shift 2
    guard "$LIVE/smoke"
    DEADDROP_HOME="$LIVE/smoke" "$ROOT"/target/debug/deaddrop send "$agent:agent:deaddrop" "$*"
    ;;
  tui) demo "$LIVE" 18803 tui ;;
  tui-klodik) demo "$LIVE" 18803 tui-klodik ;;
  status) demo "$LIVE" 18803 status ;;
  audit) demo "$LIVE" 18803 audit ;;
  down) demo "$LIVE" 18803 down ;;
  *) sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
