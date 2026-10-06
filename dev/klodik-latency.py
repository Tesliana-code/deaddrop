#!/usr/bin/env python3
"""Read a DEADDROP_TIMING_LOG and report Klodik round-trip latency by stage.

Usage: dev/klodik-latency.py <timing.log>

Stages (marks joined on the original message id):
  T0 Enter pressed in the TUI      T1 Shell::send returned
  T2 Klodik handles the inbound    T3 model process starts
  T4 model process returned        T5 signed reply sent
  T6 TUI sync has the reply        T7 reply drawn
"""
import json
import statistics
import sys

STAGES = [
    ("send (T0→T1)", "T0", "T1"),
    ("relay → agent picks up (T1→T2)", "T1", "T2"),
    ("agent receive → model start (T2→T3)", "T2", "T3"),
    ("model runtime (T3→T4)", "T3", "T4"),
    ("model return → signed send (T4→T5)", "T4", "T5"),
    ("signed send → TUI receive (T5→T6)", "T5", "T6"),
    ("TUI receive → render (T6→T7)", "T6", "T7"),
    ("total (T0→T7)", "T0", "T7"),
]


def main() -> None:
    marks: dict[str, dict[str, int]] = {}
    for line in open(sys.argv[1], encoding="utf-8"):
        m = json.loads(line)
        # The first mark of a stage wins (a retry would mark again).
        marks.setdefault(m["id"], {}).setdefault(m["stage"], m["us"])
    runs = [m for m in marks.values() if all(f"T{i}" in m for i in range(8))]
    runs.sort(key=lambda m: m["T0"])
    if not runs:
        sys.exit("no complete runs")
    ms = lambda m, a, b: (m[b] - m[a]) / 1000
    print(f"{len(runs)} complete runs (ms)\n")
    print(f"{'stage':40}" + "".join(f"{'run ' + str(i + 1):>10}" for i in range(len(runs))) + f"{'median':>10}")
    medians = {}
    for name, a, b in STAGES:
        values = [ms(m, a, b) for m in runs]
        medians[name] = statistics.median(values)
        print(f"{name:40}" + "".join(f"{v:10.0f}" for v in values) + f"{medians[name]:10.0f}")
    parts = {k: v for k, v in medians.items() if not k.startswith("total")}
    top = max(parts, key=parts.get)
    total = medians["total (T0→T7)"]
    print(f"\ndominant: {top} — {parts[top]:.0f} ms of {total:.0f} ms median total ({100 * parts[top] / total:.0f}%)")


if __name__ == "__main__":
    main()
