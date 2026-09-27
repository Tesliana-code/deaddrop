# Envelope V0 wire conformance — results

**Date:** 2026-09-27  **Base:** `c7e0fe8` (`work/core-foundation-v0-20260927`)
**Toolchains:** `results/toolchains.txt`. These are rustc 1.92.0 with serde_json 1.0.151, Python 3.12.3 and Go 1.27.1.
**Raw data:** `results/*.jsonl`. **Tables:** `results/MATRIX.md` and `results/ESCAPES.md`.

## Question

Can Envelope V0 canonical JSON be specified in a small, library-independent
normative rule set that Rust, Python and Go reproduce byte-for-byte?

## Answer

**Yes.** The rule set is `RULES.md`: seven rules, one of which is a
nine-row string-escape table.

- `python-strict` and `go-strict` each use a hand-written encoder of about 30
  lines. Neither uses a JSON encoder.
- For parsing, both use the ordinary standard-library JSON parser.
- Both agree with the production Rust implementation on every one of the
  89 vectors: 29 positive and 60 negative.
- All three also agree byte-for-byte on 30 single-code-point escape probes.
- `compare.py` exits 0 only when this agreement holds.

## Findings (measured)

| # | Finding | Evidence |
|---|---|---|
| F1 | The Rust encoder's string serialization is exactly R4: short escapes for `" \ b t n f r`, lowercase `\u00hh` for the remaining C0 characters, and raw UTF-8 for everything else, including `/`, `<>&`, DEL, C1, U+2028/2029 and non-BMP. | `ESCAPES.md` rust column |
| F2 | The Rust decoder already enforces R6. It rejects all 60 negatives. | `MATRIX.md` |
| F3 | The re-encode byte-equality rule (R6.3) neutralises parser leniency. Every accepted-negative failure caused by encoding (whitespace, order, duplicates, escape spelling, surrogates, invalid UTF-8, BOM, UTF-16, member-name case, unknown or missing members) disappears once a conforming encoder is used for the comparison. The strict profiles use the standard-library parsers unchanged. | `python-strict` and `go-strict` columns |
| F4 | **Python** `json.dumps(obj, separators=(",", ":"), ensure_ascii=False)` is byte-identical to V0 on all 29 positives and 30 probes. Its only failures are the 7 identifier-profile negatives, and those rules are absent from `PROTOCOL.md`. | `python-compact` column |
| F5 | **Python default** `json.dumps(obj)` inserts `", "` and `": "` separators and escapes every non-ASCII character and DEL (for example U+00E9, U+2028 and U+FEFF as 6-char escapes, and U+1F600 as a surrogate pair). It fails all 29 positives. | `python-default`, `ESCAPES.md` |
| F6 | **Python** `json.loads(bytes)` accepts a UTF-8 BOM, whole-document UTF-16LE, and raw UTF-8-encoded surrogates (it decodes with `surrogatepass`). Duplicate members are resolved last-wins: N07 yields body `delete everything`, and N09 yields `to` = `node-c:agent:deaddrop`. | `python-default`; side measurement |
| F7 | **Go v1** `json.Marshal` escapes `<`, `>`, `&` and U+2028/2029, so it fails P26 and P28. It encodes a nil `[]string` as `null`. | `go-default`, supplementary probe |
| F8 | **Go v1 with `SetEscapeHTML(false)`** still escapes U+2028/2029. A peer built this way that applies the spec's own re-encode rule **rejects** the canonical P26 and **accepts** the non-canonical N16. That makes two honest implementations of the current `PROTOCOL.md` accept disjoint byte strings for the same envelope. | `go-tuned` |
| F9 | **Go v1** with nil slices: `"artifact_refs":null` decodes to nil and re-encodes to `null`, so the go-tuned profile accepts N33, which Rust rejects. | `go-tuned` N33 |
| F10 | **Go v1** `Unmarshal` accepts invalid UTF-8 (overlong sequences, `FF`, truncated sequences and surrogate bytes), substituting U+FFFD. It resolves duplicates last-wins (N07 → `delete everything`) and matches member names case-insensitively (`"Body"` fills `body`). | `go-default`; side measurement |
| F11 | **Go** `encoding/json/v2`, the default in 1.27, has an encoder that matches V0 on all 29 positives and 30 probes. Its decoder rejects duplicates, invalid UTF-8 and lone surrogates. Without the byte-equality step it still accepts formatting, escape-spelling, unknown-member and missing-member variants. It matches names case-sensitively, so `"Body"` is ignored and body silently becomes `""` (N48). | `go-v2-default`; side measurement |
| F12 | The identifier constraints (R5) are enforced by Rust but not written in `PROTOCOL.md`. Every contract-following profile that has a correct encoder (`python-compact`, `go-tuned`) still accepts the identifier negatives: leading NBSP, trailing U+3000 or U+2028, U+0085, LF, trailing space, and a leading tab. This is the one remaining non-encoding interop gap. | `python-compact`, `go-tuned` |
| F13 | Supplementary side check, not in the harness: Node v20 `JSON.stringify` also matches all 29 positives and 30 probes. | ad-hoc run, 2026-09-27 |

## Where a competent engineer diverges

| Following ordinary library behaviour | Result |
|---|---|
| Python `json.dumps` with defaults | wrong bytes for every envelope |
| Python with compact separators and `ensure_ascii=False` | correct encoder |
| Go `json.Marshal` | wrong bytes when a string contains `<`, `>`, `&`, U+2028 or U+2029 |
| Go `SetEscapeHTML(false)` | still wrong for U+2028/2029, and nil slices produce `null` |
| Go `encoding/json/v2` `Marshal` | correct encoder on this corpus |
| Any parser used without the byte-equality check | accepts non-canonical input, including duplicate-member substitution of `body` or `to` |
| Any profile without R5 | accepts identifiers that Rust rejects |

## Hypotheses (not measured here)

- H1: serde_json's escaping is stable across future 1.x releases. Only
  1.0.151 was measured, and `Cargo.toml` allows any `^1.0.151`.
- H2: Other common defaults diverge, for example Java and .NET encoders
  that escape HTML-sensitive or non-ASCII characters, or C parsers that
  truncate strings at U+0000. None of these were tested.
- H3: The Python (3.12.3) and Go (1.27.1) observations hold for other
  versions of those languages.

## Treatments (proposals only; nothing here is applied)

1. Adopt `RULES.md` as normative text in `docs/PROTOCOL.md` §2 and §10,
   including R5, which closes F12.
2. Promote `vectors/envelope-v0.json` into the repository as the published V0
   conformance corpus, and make a Rust test consume it so that a serde_json
   upgrade that breaks R4 fails CI (H1).
3. State in the spec that verifiers MUST apply R6.3. F6, F10 and F11 show that
   without it, duplicate members can silently change `body` or `to` between
   what was signed (the bytes) and what is acted on (the parsed value).
4. Name the known library traps (F5, F7, F8, F9) in an implementer note.

No alternative encoding (JCS, CBOR or others) is warranted by this evidence.
RFC 8785 would change the member order and therefore every V0 byte string.
