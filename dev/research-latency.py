#!/usr/bin/env python3
"""Research runtime breakdown: where the time inside `claude -p` goes.

Runs Research's exact invocation (from the research-args example), but with
`--output-format stream-json` so each phase can be timed, in an empty temp
directory, as Research does. Records event types, tool names and monotonic
times only: never prompt, body, findings or search content.

  dev/research-latency.py [--profile deep|fast] [--runs N] [--extra '--effort low']

Marks (ms from spawn):
  R0 spawn            R1 session ready (system init)   R1b first model token
  R2 WebSearch call   R3 last WebSearch result         R4 first token after it
  R5 result event     R6 process exit
"""
import argparse
import json
import os
import statistics
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

EXTRA = ""

QUESTIONS = [
    "@research find the latest official stable Rust release",
    "@research what is the latest stable Python release?",
    "@research what is the current Node.js LTS version?",
    "@research what is the latest stable Linux kernel release?",
    "@research what is the latest PostgreSQL major release?",
]


def invocation(body: str, profile: str) -> tuple[list[str], str]:
    env = dict(os.environ, RESEARCH_PROFILE=profile, RESEARCH_EXTRA_ARGS=EXTRA)
    out = subprocess.run(
        ["cargo", "run", "-q", "-p", "deaddrop-research", "--example", "research-args",
         "--", "smoke:local:deaddrop", body],
        cwd=ROOT, env=env, check=True, capture_output=True, text=True,
    ).stdout
    d = json.loads(out)
    args = d["args"]
    i = args.index("--output-format")
    args[i + 1] = "stream-json"
    return args + ["--verbose", "--include-partial-messages"], d["stdin"]


def run(args: list[str], stdin: str) -> dict:
    marks: dict = {"tools": [], "searches": 0}
    with tempfile.TemporaryDirectory(prefix="research-latency-") as work:
        t0 = time.monotonic()
        p = subprocess.Popen(["claude", *args], cwd=work, stdin=subprocess.PIPE,
                             stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        p.stdin.write(stdin)
        p.stdin.close()
        ms = lambda: (time.monotonic() - t0) * 1000
        pending_search = set()
        for line in p.stdout:
            now = ms()
            try:
                ev = json.loads(line)
            except json.JSONDecodeError:
                continue
            kind = ev.get("type")
            if kind == "system" and ev.get("subtype") == "init":
                marks.setdefault("R1", now)
            elif kind == "stream_event":
                inner = ev.get("event", {})
                if inner.get("type") == "content_block_delta":
                    marks.setdefault("R1b", now)
                    if "R3" in marks and "R4" not in marks and not pending_search:
                        marks["R4"] = now
                if inner.get("type") == "content_block_start":
                    block = inner.get("content_block", {})
                    marks.setdefault("blocks", []).append(block.get("type"))
                    if block.get("type") in ("tool_use", "server_tool_use"):
                        marks["tools"].append(block.get("name"))
                        if block.get("name") == "StructuredOutput":
                            marks.setdefault("R4s", now)
            elif kind == "assistant":
                for block in ev.get("message", {}).get("content", []):
                    if block.get("type") == "tool_use" and block.get("name") == "WebSearch":
                        marks.setdefault("R2", now)
                        marks["searches"] += 1
                        pending_search.add(block.get("id"))
            elif kind == "user":
                for block in ev.get("message", {}).get("content", []) or []:
                    if isinstance(block, dict) and block.get("tool_use_id") in pending_search:
                        pending_search.discard(block["tool_use_id"])
                        marks["R3"] = now
                        marks.pop("R4", None)
            elif kind == "result":
                marks["R5"] = now
                marks["ok"] = ev.get("subtype") == "success" and not ev.get("is_error")
                so = ev.get("structured_output") or {}
                srcs = so.get("sources") or []
                marks["sources"] = len(srcs)
                marks["urls"] = [s.get("url", "") for s in srcs]
                marks["findings"] = so.get("findings", "")
                marks["turns"] = ev.get("num_turns")
                marks["api_ms"] = ev.get("duration_api_ms")
                marks["out_tokens"] = (ev.get("usage") or {}).get("output_tokens")
        p.wait()
        marks["R6"] = ms()
    return marks


def median(xs):
    xs = [x for x in xs if x is not None]
    return f"{statistics.median(xs):7.0f}" if xs else "      -"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--profile", default="current")
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--json", help="also write per-run marks here (no content)")
    ap.add_argument("--keep-findings", action="store_true",
                    help="keep findings and source URLs in --json, for a parity review")
    ap.add_argument("--extra", default="", help="extra CLI flags, e.g. '--effort low'")
    a = ap.parse_args()
    global EXTRA
    EXTRA = a.extra
    runs = []
    for i in range(a.runs):
        q = QUESTIONS[i % len(QUESTIONS)]
        args, stdin = invocation(q, a.profile)
        m = run(args, stdin)
        m["q"] = i % len(QUESTIONS)
        runs.append(m)
        print(f"run {i + 1}: total {m['R6']:.0f} ms, ok {m.get('ok')}, searches {m['searches']}, "
              f"sources {m.get('sources')}, tools {m['tools']}, blocks {m.get('blocks')}", file=sys.stderr)
    d = lambda r, a, b: (r[b] - r[a]) if a in r and b in r else None
    rows = [
        ("process startup  R0→R1", lambda r: r.get("R1")),
        ("first token      R1→R1b", lambda r: d(r, "R1", "R1b")),
        ("pre-search       R1→R2", lambda r: d(r, "R1", "R2")),
        ("WebSearch        R2→R3", lambda r: d(r, "R2", "R3")),
        ("synthesis        R3→R5", lambda r: d(r, "R3", "R5")),
        ("  of which first token R3→R4", lambda r: d(r, "R3", "R4")),
        ("  until StructuredOutput R3→R4s", lambda r: d(r, "R3", "R4s")),
        ("  StructuredOutput→result", lambda r: d(r, "R4s", "R5")),
        ("shutdown         R5→R6", lambda r: d(r, "R5", "R6")),
        ("TOTAL            R0→R6", lambda r: r["R6"]),
    ]
    print(f"\nprofile {a.profile} {a.extra}: {len(runs)} runs (ms)")
    for name, f in rows:
        vals = [f(r) for r in runs]
        print(f"  {name:30}" + "".join(f"{v:8.0f}" if v is not None else "       -" for v in vals)
              + f"  median {median(vals)}")
    print(f"  {'searches':30}" + "".join(f"{r['searches']:8}" for r in runs))
    print(f"  {'sources':30}" + "".join(f"{r.get('sources', 0):8}" for r in runs))
    print(f"  {'output tokens':30}" + "".join(f"{r.get('out_tokens') or 0:8}" for r in runs))
    print(f"  {'ok':30}" + "".join(f"{str(r.get('ok')):>8}" for r in runs))
    if not a.keep_findings:
        for r in runs:
            r.pop("findings", None)
            r.pop("urls", None)
    if a.json:
        with open(a.json, "w") as f:
            json.dump(runs, f)


if __name__ == "__main__":
    main()
