#!/usr/bin/env bash
# Reproduce the Envelope V0 wire conformance experiment end to end.
#
# Requirements: cargo (repo toolchain), python3 >= 3.9, go >= 1.25
# (encoding/json/v2 must be importable; set GO=/path/to/go if not on PATH).
set -euo pipefail

cd "$(dirname "$0")"
GO="${GO:-go}"
export GOTOOLCHAIN=local

python3 gen_vectors.py
git diff --quiet -- vectors/ 2>/dev/null || echo "note: regenerated corpus differs from committed copy"

mkdir -p results
rm -f results/*.jsonl

(cd rust && cargo run --quiet --locked --offline -- ../vectors/envelope-v0.json) \
  > results/rust-reference.jsonl
python3 python/conformance.py vectors/envelope-v0.json > results/python.jsonl
(cd go && "$GO" run . ../vectors/envelope-v0.json) > results/go.jsonl

{
  echo "rustc: $(rustc --version)"
  echo "serde_json: $(grep -A1 'name = "serde_json"' rust/Cargo.lock | sed -n 's/version = //p' | tr -d '"')"
  echo "python: $(python3 --version 2>&1)"
  echo "go: $("$GO" version)"
} > results/toolchains.txt

python3 compare.py
