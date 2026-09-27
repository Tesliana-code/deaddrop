# Experiment: Envelope V0 wire conformance

This experiment tests whether the canonical Envelope V0 JSON encoding can be
specified independently of Rust/serde_json and reproduced byte-for-byte by
other languages.

It is isolated research. It does not change the protocol, storage or
production crates.

- Results and the answer: [`RESULTS.md`](RESULTS.md)
- Draft rule set: [`RULES.md`](RULES.md)
- Generated tables: [`results/MATRIX.md`](results/MATRIX.md), [`results/ESCAPES.md`](results/ESCAPES.md)

## Layout

| path | role |
|---|---|
| `gen_vectors.py` | builds the corpus. Positive bytes are hand-assembled literals; no JSON encoder is involved |
| `vectors/envelope-v0.json` | language-neutral corpus: 29 positive and 60 negative vectors, with exact wire bytes as hex and semantic fields as UTF-8 hex |
| `rust/` | reference observation of the production `deaddrop-protocol` encoder and decoder |
| `python/conformance.py` | profiles `python-default`, `python-compact` and `python-strict` |
| `go/main.go` | profiles `go-default` (json v1), `go-v2-default`, `go-tuned` and `go-strict` |
| `compare.py` | renders the tables. Fails unless rust, python-strict and go-strict agree |
| `run.sh` | runs the whole experiment end to end |

## Pass criteria

- **Positive vector:** the encoder reproduces `wire_hex` exactly, and the
  decoder accepts it and returns the identical fields.
- **Negative vector:** the decoder rejects it.

Default and tuned profiles are observations of ordinary library behaviour.
Their failures are data, not harness failures.

## Run

```sh
GO=/path/to/go experiments/wire-conformance-v0/run.sh
```

Requirements:
- the repository Rust toolchain (the run uses `--offline --locked`, so crates must be cached)
- Python 3.9 or later
- Go 1.25 or later, with `encoding/json/v2` importable (1.27.1 was used)
