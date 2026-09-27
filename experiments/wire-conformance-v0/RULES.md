# Envelope V0 canonical JSON — draft normative rule set

**Status:** PROPOSAL from `experiments/wire-conformance-v0`. Not normative
until adopted into `docs/PROTOCOL.md`. It describes the behaviour of the
current Rust implementation (`deaddrop-protocol`, serde_json 1.0.151) and does
not change it.

Evidence: `python-strict` and `go-strict` implement these rules with no JSON
encoder, and they agree byte-for-byte with the Rust reference on all
89 corpus vectors and 30 escape probes (`results/MATRIX.md`,
`results/ESCAPES.md`).

## R1 Document

The wire form is the UTF-8 bytes of exactly one JSON object. There is no byte
order mark, no whitespace outside strings, and nothing before or after the
object.

## R2 Members

There are exactly eight members, in this order, with these literal names:

```text
protocol, id, from, to, kind, correlation_id, body, artifact_refs
```

Members are separated by `,`. Each name is followed immediately by `:`.

## R3 Values

| member | value |
|---|---|
| `protocol` | the string `deaddrop/0` |
| `id`, `from`, `to` | identifier string (R5) |
| `kind` | one of `message`, `request`, `response`, `handoff`, `task_claim`, `checkpoint`, `acknowledgment`, `error`, `capability_declaration` |
| `correlation_id` | identifier string (R5), or the literal `null` if and only if absent |
| `body` | string |
| `artifact_refs` | array of strings matching `sha256:[0-9a-f]{64}`, in sender order, with duplicates preserved. It is `[]` when empty, never `null`, and elements are separated by `,` |

No numbers, booleans or nested objects occur anywhere.

## R4 String serialization

A string value is a sequence of Unicode scalar values. Surrogate code points
cannot be represented. The serialization is `"`, then each scalar in order
as follows, then `"`:

| scalar | bytes |
|---|---|
| U+0022 quotation mark | backslash, `"` |
| U+005C reverse solidus | backslash, backslash |
| U+0008 | backslash, `b` |
| U+0009 | backslash, `t` |
| U+000A | backslash, `n` |
| U+000C | backslash, `f` |
| U+000D | backslash, `r` |
| any other U+0000..U+001F | backslash, `u`, `0`, `0`, two **lowercase** hex digits |
| every other scalar | its UTF-8 encoding, unescaped |

"Every other scalar" explicitly includes `/`, `<`, `>`, `&`, `'`, U+007F, the
C1 controls U+0080..U+009F, U+2028, U+2029, U+FEFF, noncharacters and all
non-BMP scalars (written as 4-byte UTF-8, never as surrogate-pair escapes).

No Unicode normalization is applied. NFC and NFD spellings are different
strings.

This is the ECMAScript `JSON.stringify` / RFC 8785 string serialization for
well-formed strings. It is **not** RFC 8785 as a whole, because R2 fixes the
member order instead of sorting it.

## R5 Identifier profile (`id`, `from`, `to`, `correlation_id`)

- non-empty
- the first and the last scalar are not Unicode `White_Space`:
  U+0009..U+000D, U+0020, U+0085, U+00A0, U+1680, U+2000..U+200A, U+2028,
  U+2029, U+202F, U+205F, U+3000
- no scalar in general category Cc: U+0000..U+001F, U+007F..U+009F

Interior spaces, non-ASCII, format characters (for example U+200B, U+202E)
and unnormalized text are accepted by V0. The last two are review
findings, not endorsements.

## R6 Acceptance

A receiver accepts bytes `B` if and only if all of the following hold:

1. `B` is well-formed UTF-8.
2. `B` parses as RFC 8259 JSON into a value that satisfies R2, R3 and R5.
3. `Encode(value)`, per R1 to R4, is byte-for-byte equal to `B`.

Step 3 carries the canonicality guarantee. When `B` equals an R1 to R4
encoding, `B` is by construction free of duplicates, alternative escapes,
whitespace and invalid UTF-8. The parser in step 2 therefore only has to be
correct on canonical documents, and any RFC 8259 parser qualifies. Duplicate
handling, escape decoding, U+FFFD substitution, case-insensitive member
matching and BOM tolerance in the parser become irrelevant.

## R7 Encoder obligation

An encoder MUST refuse, rather than emit, a value that violates R3 or R5, or
that contains a surrogate code point.
