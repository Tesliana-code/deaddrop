#!/usr/bin/env python3
"""Generate the language-neutral Envelope V0 wire conformance corpus.

Expected wire bytes for positive vectors are hand-assembled from JSON
literal text written in this file. No JSON encoder is used to produce an
expectation. Every implementation under test must reproduce these bytes.

This source file is deliberately ASCII-only. Backslashes are spelled via
BS so that no escape sequence is ever interpreted by tooling.

Output: vectors/envelope-v0.json (ASCII-only JSON).
"""

import json
import pathlib
import re

BS = chr(92)
Q = chr(34)

A64 = "a" * 64
B64 = "b" * 64
C64 = "c" * 64
REF_A = "sha256:" + A64
REF_B = "sha256:" + B64
REF_C = "sha256:" + C64

KINDS = [
    "message",
    "request",
    "response",
    "handoff",
    "task_claim",
    "checkpoint",
    "acknowledgment",
    "error",
    "capability_declaration",
]

GOLDEN_FIELDS = {
    "id": "msg-wire-1",
    "from": "node-a:agent:deaddrop",
    "to": "node-b:agent:deaddrop",
    "kind": "handoff",
    "correlation_id": "corr-wire-1",
    "body": "continue this task",
    "artifact_refs": [REF_A],
}

TRIVIAL = re.compile(r"[A-Za-z0-9 :._/<>&+-]*")


def lit(value):
    """Trivial quoting for strings that need no escaping at all."""
    assert TRIVIAL.fullmatch(value), value
    return Q + value + Q


def raw(value):
    """Literal containing raw (unescaped) characters, UTF-8 on the wire."""
    assert Q not in value and BS not in value
    assert all(ord(c) >= 0x20 for c in value), value
    return Q + value + Q


def assemble(fields, lits=None, protocol_lit=None):
    """Assemble wire text from literal pieces in the frozen V0 order."""
    lits = lits or {}

    def piece(name):
        if name in lits:
            return lits[name]
        value = fields[name]
        if value is None:
            return "null"
        return lit(value)

    refs = lits.get("artifact_refs")
    if refs is None:
        refs = "[" + ",".join(lit(r) for r in fields["artifact_refs"]) + "]"

    parts = [
        '"protocol":' + (protocol_lit or lit("deaddrop/0")),
        '"id":' + piece("id"),
        '"from":' + piece("from"),
        '"to":' + piece("to"),
        '"kind":' + piece("kind"),
        '"correlation_id":' + piece("correlation_id"),
        '"body":' + piece("body"),
        '"artifact_refs":' + refs,
    ]
    return "{" + ",".join(parts) + "}"


def golden(**overrides):
    fields = dict(GOLDEN_FIELDS)
    fields.update(overrides)
    return fields


def hexs(value):
    return value.encode("utf-8").hex()


vectors = []


def positive(vid, note, fields, lits=None):
    wire = assemble(fields, lits).encode("utf-8")
    vectors.append(
        {
            "id": vid,
            "class": "positive",
            "note": note,
            "fields": fields,
            "fields_utf8_hex": {
                k: (
                    None
                    if v is None
                    else [hexs(x) for x in v]
                    if isinstance(v, list)
                    else hexs(v)
                )
                for k, v in fields.items()
            },
            "wire_hex": wire.hex(),
        }
    )


def negative(vid, note, wire):
    if isinstance(wire, str):
        wire = wire.encode("utf-8")
    vectors.append(
        {"id": vid, "class": "negative", "note": note, "wire_hex": wire.hex()}
    )


# ---------------------------------------------------------------- positives

positive("P01-golden-ascii", "existing Rust golden vector", golden())
positive("P02-correlation-null", "absent correlation is explicit null",
         golden(correlation_id=None))
positive("P03-empty-body", "empty body string", golden(body=""))
positive("P04-refs-ordered-unsorted", "artifact ref order preserved, not sorted",
         golden(artifact_refs=[REF_B, REF_A, REF_C]))
positive("P05-refs-duplicate", "duplicate artifact refs are preserved",
         golden(artifact_refs=[REF_A, REF_A, REF_B]))
positive("P06-refs-empty", "empty artifact ref array",
         golden(artifact_refs=[]))

for n, kind in enumerate(KINDS, start=1):
    positive(f"P1{n}-kind-{kind}", f"message kind {kind}",
             golden(kind=kind, correlation_id=None, artifact_refs=[]))

body = "say " + Q + "hi" + Q + " " + BS + " end"
positive("P20-quote-backslash", "quote and backslash use two-char escapes",
         golden(body=body),
         {"body": Q + "say " + BS + Q + "hi" + BS + Q + " " + BS + BS + " end" + Q})

body = chr(8) + chr(9) + chr(10) + chr(12) + chr(13)
positive("P21-short-escapes", "BS HT LF FF CR use short escapes",
         golden(body=body),
         {"body": Q + BS + "b" + BS + "t" + BS + "n" + BS + "f" + BS + "r" + Q})

body = chr(0) + chr(1) + chr(0x0B) + chr(0x0E) + chr(0x1F)
positive("P22-c0-u-escapes", "other C0 controls use lowercase 6-char escapes",
         golden(body=body),
         {"body": Q + BS + "u0000" + BS + "u0001" + BS + "u000b"
          + BS + "u000e" + BS + "u001f" + Q})

body = chr(0x7F) + chr(0x80) + chr(0x85) + chr(0x9F)
positive("P23-del-c1-raw", "DEL and C1 controls are raw UTF-8",
         golden(body=body), {"body": raw(body)})

body = "caf" + chr(0xE9)
positive("P24-nfc", "precomposed U+00E9 raw UTF-8", golden(body=body),
         {"body": raw(body)})

body = "cafe" + chr(0x301)
positive("P25-nfd", "decomposed e+U+0301 raw UTF-8, not normalized",
         golden(body=body), {"body": raw(body)})

body = "a" + chr(0x2028) + "b" + chr(0x2029) + "c"
positive("P26-u2028-u2029", "line/paragraph separators raw UTF-8",
         golden(body=body), {"body": raw(body)})

body = "ok " + chr(0x1F600)
positive("P27-emoji", "non-BMP scalar is raw 4-byte UTF-8", golden(body=body),
         {"body": raw(body)})

positive("P28-html-chars", "< > & are raw", golden(body="<b> & </b>"))
positive("P29-solidus", "solidus is raw", golden(body="a/b//c"))

body = chr(0xFEFF) + "x" + chr(0xFFFF) + chr(0xFFFD)
positive("P30-bom-nonchar-in-body", "U+FEFF U+FFFF U+FFFD inside a string are raw",
         golden(body=body), {"body": raw(body)})

fields = golden(id="msg with space", **{"from": "n" + chr(0xF6) + "de-a"})
positive("P31-nonascii-ids", "identifiers may contain interior space and non-ASCII",
         fields, {"from": raw(fields["from"])})

positive("P32-self-addressed", "from == to is accepted by V0",
         golden(to="node-a:agent:deaddrop"))

body = "line one" + chr(10) + "tab" + chr(9) + "quote " + Q + " " + chr(0xE9) + chr(0x1F600)
positive("P33-mixed", "mixed escapes and raw UTF-8 in one string",
         golden(body=body),
         {"body": Q + "line one" + BS + "n" + "tab" + BS + "t" + "quote " + BS + Q
          + " " + chr(0xE9) + chr(0x1F600) + Q})

# ---------------------------------------------------------------- negatives

G = assemble(golden())
GOLDEN_OBJ = golden()


def with_body_lit(body_lit):
    return assemble(golden(), {"body": body_lit})


def with_body_bytes(body_bytes):
    head, tail = G.split('"body":"continue this task"')
    return head.encode() + b'"body":"' + body_bytes + b'"' + tail.encode()


negative("N01-field-reordered", "protocol and id swapped",
         G.replace('"protocol":"deaddrop/0","id":"msg-wire-1"',
                   '"id":"msg-wire-1","protocol":"deaddrop/0"'))
negative("N02-pretty", "pretty-printed with 2-space indent",
         json.dumps({"protocol": "deaddrop/0", **GOLDEN_OBJ}, indent=2))
negative("N03-trailing-newline", "canonical bytes plus LF", G + chr(10))
negative("N04-leading-space", "leading space", " " + G)
negative("N05-space-after-colon", "space after one colon",
         G.replace('"kind":"handoff"', '"kind": "handoff"'))
negative("N06-dup-key-same", "duplicate body key, same value",
         G.replace('"body":"continue this task"',
                   '"body":"continue this task","body":"continue this task"'))
negative("N07-dup-key-diff", "duplicate body key, different value",
         G.replace('"body":"continue this task"',
                   '"body":"continue this task","body":"delete everything"'))
negative("N08-dup-protocol", "duplicate protocol key",
         G.replace('"protocol":"deaddrop/0"',
                   '"protocol":"deaddrop/0","protocol":"deaddrop/0"'))
negative("N09-dup-to-diff", "duplicate recipient, second differs",
         G.replace('"to":"node-b:agent:deaddrop"',
                   '"to":"node-b:agent:deaddrop","to":"node-c:agent:deaddrop"'))
negative("N10-u0041", "A spelled as a 6-char escape",
         with_body_lit(Q + BS + "u0041" + Q))
negative("N11-escaped-solidus", "solidus escaped", with_body_lit(Q + "a" + BS + "/b" + Q))
negative("N12-uppercase-hex", "uppercase hex in C0 escape",
         with_body_lit(Q + BS + "u001F" + Q))
negative("N13-escaped-nonascii", "e-acute escaped (Python ensure_ascii)",
         with_body_lit(Q + "caf" + BS + "u00e9" + Q))
negative("N14-escaped-lt", "< escaped (Go HTML-safe default)",
         with_body_lit(Q + BS + "u003cb" + BS + "u003e" + Q))
negative("N15-escaped-amp", "& escaped (Go HTML-safe default)",
         with_body_lit(Q + "a " + BS + "u0026 b" + Q))
negative("N16-escaped-u2028", "U+2028 escaped (Go default)",
         with_body_lit(Q + "a" + BS + "u2028b" + Q))
negative("N17-surrogate-pair-escape", "emoji as escaped surrogate pair",
         with_body_lit(Q + "ok " + BS + "ud83d" + BS + "ude00" + Q))
negative("N18-lone-high-surrogate", "escaped lone high surrogate",
         with_body_lit(Q + BS + "ud800" + Q))
negative("N19-lone-low-surrogate", "escaped lone low surrogate",
         with_body_lit(Q + BS + "udc00" + Q))
negative("N20-u000a-for-newline", "LF spelled as 6-char escape instead of short",
         with_body_lit(Q + BS + "u000a" + Q))
negative("N21-escaped-del", "DEL escaped", with_body_lit(Q + BS + "u007f" + Q))
negative("N22-raw-c0", "raw U+0001 inside string (invalid JSON)",
         with_body_bytes(b"\x01"))
negative("N23-overlong-utf8", "overlong encoding C0 AF", with_body_bytes(b"\xc0\xaf"))
negative("N24-raw-surrogate-utf8", "UTF-8 encoded surrogate ED A0 80",
         with_body_bytes(b"\xed\xa0\x80"))
negative("N25-invalid-byte-ff", "byte FF", with_body_bytes(b"\xff"))
negative("N26-truncated-utf8", "truncated 3-byte sequence E2 82",
         with_body_bytes(b"\xe2\x82"))
negative("N27-bom-prefix", "UTF-8 BOM before object", b"\xef\xbb\xbf" + G.encode())
negative("N28-utf16le", "whole document as UTF-16LE", G.encode("utf-16-le"))
negative("N29-unknown-field", "extra member",
         G[:-1] + ',"surprise":"field"}')
negative("N30-missing-correlation", "correlation_id member omitted",
         G.replace('"correlation_id":"corr-wire-1",', ""))
negative("N31-missing-artifact-refs", "artifact_refs member omitted",
         G.replace(',"artifact_refs":["' + REF_A + '"]', ""))
negative("N32-correlation-empty", "correlation_id empty string",
         assemble(golden(correlation_id=None), {"correlation_id": '""'}))
negative("N33-artifact-refs-null", "artifact_refs null",
         assemble(golden(), {"artifact_refs": "null"}))
negative("N34-body-null", "body null", assemble(golden(), {"body": "null"}))
negative("N35-body-number", "body is a number", assemble(golden(), {"body": "42"}))
negative("N36-protocol-v1", "wrong protocol version",
         assemble(golden(), protocol_lit=lit("deaddrop/1")))
negative("N37-protocol-case", "protocol case variant",
         assemble(golden(), protocol_lit=lit("Deaddrop/0")))
negative("N38-protocol-00", "protocol deaddrop/00",
         assemble(golden(), protocol_lit=lit("deaddrop/00")))
negative("N39-kind-case", "kind Message", assemble(golden(kind="Message")))
negative("N40-kind-hyphen", "kind task-claim", assemble(golden(kind="task-claim")))
negative("N41-kind-camel", "kind taskClaim", assemble(golden(kind="taskClaim")))
negative("N42-ref-uppercase", "uppercase digest",
         assemble(golden(artifact_refs=["sha256:" + "A" * 64])))
negative("N43-ref-plus-sign", "digest pairs with + sign",
         assemble(golden(artifact_refs=["sha256:" + "+f" * 32])))
negative("N44-ref-scheme-case", "SHA256: prefix",
         assemble(golden(artifact_refs=["SHA256:" + A64])))
negative("N45-ref-63", "63 hex chars", assemble(golden(artifact_refs=["sha256:" + "a" * 63])))
negative("N46-ref-65", "65 hex chars", assemble(golden(artifact_refs=["sha256:" + "a" * 65])))
negative("N47-ref-bare-hex", "digest without scheme", assemble(golden(artifact_refs=[A64])))
negative("N48-key-case", "member name Body (Go case-insensitive match)",
         G.replace('"body":', '"Body":'))
negative("N49-key-escaped", "member name with escaped letter",
         G.replace('"body":', '"' + BS + "u0062ody" + '":'))
negative("N50-id-empty", "empty id", assemble(golden(id=""), {"id": '""'}))
negative("N51-id-leading-nbsp", "id with leading U+00A0",
         assemble(golden(), {"id": raw(chr(0xA0) + "msg")}))
negative("N52-id-trailing-ideographic-space", "id with trailing U+3000",
         assemble(golden(), {"id": raw("msg" + chr(0x3000))}))
negative("N53-id-c1-nel", "id containing U+0085 (Cc and White_Space)",
         assemble(golden(), {"id": raw("m" + chr(0x85) + "g")}))
negative("N54-id-escaped-lf", "id containing LF", assemble(golden(), {"id": Q + "m" + BS + "ng" + Q}))
negative("N55-from-trailing-space", "from with trailing ASCII space",
         assemble(golden(), {"from": '"node-a "'}))
negative("N56-correlation-leading-tab", "correlation id with leading tab",
         assemble(golden(), {"correlation_id": Q + BS + "tcorr" + Q}))
negative("N57-id-trailing-u2028", "id with trailing U+2028 (White_Space, not Cc)",
         assemble(golden(), {"id": raw("msg" + chr(0x2028))}))
negative("N58-top-level-array", "top-level array", '["deaddrop/0"]')
negative("N59-json-null", "JSON null", "null")
negative("N60-empty-input", "zero bytes", b"")

ESCAPE_PROBE = [
    0x00, 0x01, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x1F, 0x20, 0x22, 0x26,
    0x27, 0x2F, 0x3C, 0x3E, 0x5C, 0x7F, 0x80, 0x85, 0x9F, 0xA0, 0xE9, 0x301,
    0x2028, 0x2029, 0xFEFF, 0xFFFD, 0xFFFF, 0x1F600,
]

ids = [v["id"] for v in vectors]
assert len(ids) == len(set(ids)), "duplicate vector id"

corpus = {
    "corpus": "deaddrop-envelope-v0-wire-conformance",
    "revision": 1,
    "field_order": ["protocol", "id", "from", "to", "kind", "correlation_id",
                    "body", "artifact_refs"],
    "message_kinds": KINDS,
    "golden_fields": GOLDEN_FIELDS,
    "escape_probe_codepoints": ESCAPE_PROBE,
    "vectors": vectors,
}

out = pathlib.Path(__file__).parent / "vectors" / "envelope-v0.json"
out.parent.mkdir(exist_ok=True)
out.write_text(json.dumps(corpus, indent=1, ensure_ascii=True) + "\n")
print(f"wrote {out} ({sum(v['class'] == 'positive' for v in vectors)} positive, "
      f"{sum(v['class'] == 'negative' for v in vectors)} negative)")
