#!/usr/bin/env python3
"""Room request latency by agent, from a DEADDROP_TIMING_LOG.

Usage: dev/room-latency.py <timing.log>

Marks are joined on the logical room message id of the request:
  T0 Enter pressed        T1 fan-out published     T2 agent picks it up (verified)
  T3 runtime starts       T4 runtime returns       T5 signed report published
  T6 human TUI has it     T7 drawn
"""
import json
import statistics
import sys

STAGES = [
    ("send/fan-out T0→T1", "T0", "T1"),
    ("transport+agent poll T1→T2", "T1", "T2"),
    ("agent dispatch T2→T3", "T2", "T3"),
    ("runtime T3→T4", "T3", "T4"),
    ("report sign/send T4→T5", "T4", "T5"),
    ("transport+TUI sync T5→T6", "T5", "T6"),
    ("render T6→T7", "T6", "T7"),
    ("TOTAL T0→T7", "T0", "T7"),
]


def main() -> None:
    marks: dict[str, dict] = {}
    for line in open(sys.argv[1], encoding="utf-8"):
        m = json.loads(line)
        run = marks.setdefault(m["id"], {})
        run.setdefault(m["stage"], m["us"])
        if m["stage"] == "T2":
            run["agent"] = m["who"].split(":")[0]
    runs = [r for r in marks.values() if all(f"T{i}" in r for i in range(8))]
    by_agent: dict[str, list] = {}
    for r in sorted(runs, key=lambda r: r["T0"]):
        by_agent.setdefault(r["agent"], []).append(r)
    ms = lambda r, a, b: (r[b] - r[a]) / 1000
    for agent, rs in by_agent.items():
        print(f"\n{agent}: {len(rs)} runs (ms)")
        print(f"  {'stage':30}" + "".join(f"{'#' + str(i + 1):>8}" for i in range(len(rs))) + f"{'median':>9}")
        for name, a, b in STAGES:
            vals = [ms(r, a, b) for r in rs]
            print(f"  {name:30}" + "".join(f"{v:8.0f}" for v in vals) + f"{statistics.median(vals):9.0f}")
        infra = [ms(r, "T0", "T7") - ms(r, "T3", "T4") for r in rs]
        print(f"  {'INFRASTRUCTURE (total−runtime)':30}" + "".join(f"{v:8.0f}" for v in infra) + f"{statistics.median(infra):9.0f}")


if __name__ == "__main__":
    main()
