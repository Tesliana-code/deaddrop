#!/usr/bin/env bash
# Measure room request latency through the real TUI, as the smoke identity
# (never the human's). Fresh smoke network with DEADDROP_TIMING_LOG on, then
# N requests to each agent via the TUI room composer, then the report.
#
#   dev/room-latency-run.sh [runs-per-agent]     (default 5)
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
N=${1:-5}
LOG=$ROOT/target/agent-bus-smoke/timing-$(date +%Y%m%dT%H%M%S).log
export DEADDROP_TIMING_LOG=$LOG
T="tmux -L latency"

"$ROOT"/dev/agent-bus-smoke.sh down >/dev/null 2>&1 || true
"$ROOT"/dev/agent-bus-smoke.sh up >/dev/null
$T kill-server 2>/dev/null || true
$T -f /dev/null new-session -d -s s -x 140 -y 40 -e DEADDROP_TIMING_LOG="$LOG" \
  "$ROOT/dev/agent-bus-smoke.sh tui; sleep 600"
sleep 3

drawn() { if [ -f "$LOG" ]; then grep -c '"T7"' "$LOG" || true; else echo 0; fi; }
ask() {
  local before; before=$(drawn)
  $T send-keys -t s -l "$1"; $T send-keys -t s Enter
  for _ in $(seq 240); do [ "$(drawn)" -gt "${before:-0}" ] && return; sleep 0.25; done
  echo "timed out: $1" >&2
}

for i in $(seq "$N"); do
  ask "@klodik ping $i, answer with one short sentence"
  ask "@github does commit 154f992cc3e88f51a0b6bdbf42998e94c3aeedaf modify crates/deaddrop-tui/src/ui.rs?"
  ask "@research what is the latest stable Rust version? one sentence with one source"
done

$T send-keys -t s Escape; $T send-keys -t s C-c; sleep 0.5; $T kill-server 2>/dev/null || true
echo "log: $LOG"
python3 "$ROOT"/dev/room-latency.py "$LOG"
