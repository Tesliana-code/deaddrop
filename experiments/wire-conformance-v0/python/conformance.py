#!/usr/bin/env python3
"""Python observations for the Envelope V0 wire conformance corpus.

Profiles:

  python-default  json.dumps(obj) / json.loads + contract-level validation.
                  What ordinary library use produces. No byte comparison.
  python-compact  json.dumps(separators=(",", ":"), ensure_ascii=False),
                  i.e. a literal reading of "compact UTF-8 JSON", plus the
                  PROTOCOL.md re-encode byte-equality acceptance rule.
  python-strict   hand-written encoder from RULES.md (no JSON encoder used),
                  ordinary json.loads for parsing, draft identifier profile,
                  and re-encode byte equality.

Emits one JSON line per (profile, vector) and per escape probe on stdout.
This source is ASCII-only; the backslash is spelled BS.
"""

import json
import re
import sys

BS = chr(92)
Q = chr(34)
FIELDS = ["protocol", "id", "from", "to", "kind", "correlation_id", "body", "artifact_refs"]
KINDS = {
    "message", "request", "response", "handoff", "task_claim", "checkpoint",
    "acknowledgment", "error", "capability_declaration",
}
REF = re.compile(r"sha256:[0-9a-f]{64}")

# RULES.md R-ID: Unicode White_Space (identical to Rust char::is_whitespace).
WHITE_SPACE = set(range(0x09, 0x0E)) | {0x20, 0x85, 0xA0, 0x1680} | set(
    range(0x2000, 0x200B)) | {0x2028, 0x2029, 0x202F, 0x205F, 0x3000}


class Reject(Exception):
    pass


# ------------------------------------------------------------ validation


def is_scalar_text(s):
    return not any(0xD800 <= ord(c) <= 0xDFFF for c in s)


def valid_id_contract(s):
    return isinstance(s, str) and s != ""


def valid_id_strict(s):
    if not isinstance(s, str) or s == "" or not is_scalar_text(s):
        return False
    if ord(s[0]) in WHITE_SPACE or ord(s[-1]) in WHITE_SPACE:
        return False
    return not any(ord(c) < 0x20 or 0x7F <= ord(c) <= 0x9F for c in s)


def validate(obj, valid_id):
    if not isinstance(obj, dict):
        raise Reject("not an object")
    for name in FIELDS:
        if name not in obj:
            raise Reject(f"missing {name}")
    if obj["protocol"] != "deaddrop/0":
        raise Reject("protocol")
    for name in ("id", "from", "to"):
        if not valid_id(obj[name]):
            raise Reject(f"invalid {name}")
    if obj["kind"] not in KINDS:
        raise Reject("kind")
    if obj["correlation_id"] is not None and not valid_id(obj["correlation_id"]):
        raise Reject("invalid correlation_id")
    if not isinstance(obj["body"], str):
        raise Reject("body type")
    refs = obj["artifact_refs"]
    if not isinstance(refs, list) or not all(isinstance(r, str) and REF.fullmatch(r) for r in refs):
        raise Reject("artifact_refs")
    return {name: obj[name] for name in FIELDS if name != "protocol"}


def ordered(fields):
    return {"protocol": "deaddrop/0", **{k: fields[k] for k in FIELDS[1:]}}


# ------------------------------------------------------------ encoders


def encode_default(fields):
    return json.dumps(ordered(fields)).encode("utf-8")


def encode_compact(fields):
    return json.dumps(ordered(fields), separators=(",", ":"), ensure_ascii=False).encode("utf-8")


SHORT = {0x08: "b", 0x09: "t", 0x0A: "n", 0x0C: "f", 0x0D: "r", 0x22: Q, 0x5C: BS}
HEX = "0123456789abcdef"


def strict_string(s):
    out = [Q]
    for ch in s:
        c = ord(ch)
        if 0xD800 <= c <= 0xDFFF:
            raise Reject("surrogate code point")
        if c in SHORT:
            out.append(BS + SHORT[c])
        elif c < 0x20:
            out.append(BS + "u00" + HEX[c >> 4] + HEX[c & 15])
        else:
            out.append(ch)
    out.append(Q)
    return "".join(out)


def encode_strict(fields):
    corr = fields["correlation_id"]
    parts = [
        '"protocol":' + strict_string("deaddrop/0"),
        '"id":' + strict_string(fields["id"]),
        '"from":' + strict_string(fields["from"]),
        '"to":' + strict_string(fields["to"]),
        '"kind":' + strict_string(fields["kind"]),
        '"correlation_id":' + ("null" if corr is None else strict_string(corr)),
        '"body":' + strict_string(fields["body"]),
        '"artifact_refs":[' + ",".join(strict_string(r) for r in fields["artifact_refs"]) + "]",
    ]
    return ("{" + ",".join(parts) + "}").encode("utf-8")


# ------------------------------------------------------------ decoders


def decode_default(wire):
    try:
        obj = json.loads(wire)
    except (ValueError, UnicodeDecodeError) as e:
        raise Reject(f"json: {e}")
    return validate(obj, valid_id_contract)


def decode_compact(wire):
    fields = decode_default(wire)
    try:
        again = encode_compact(fields)
    except UnicodeEncodeError as e:
        raise Reject(f"re-encode: {e}")
    if again != wire:
        raise Reject("non-canonical")
    return fields


def no_duplicates(pairs):
    seen = {}
    for key, value in pairs:
        if key in seen:
            raise Reject(f"duplicate member {key}")
        seen[key] = value
    return seen


def decode_strict(wire):
    try:
        text = wire.decode("utf-8")
        obj = json.loads(text, object_pairs_hook=no_duplicates)
    except (ValueError, UnicodeDecodeError) as e:
        raise Reject(f"json: {e}")
    if not isinstance(obj, dict) or set(obj) != set(FIELDS):
        raise Reject("member set")
    fields = validate(obj, valid_id_strict)
    if encode_strict(fields) != wire:
        raise Reject("non-canonical")
    return fields


PROFILES = {
    "python-default": (encode_default, decode_default),
    "python-compact": (encode_compact, decode_compact),
    "python-strict": (encode_strict, decode_strict),
}


# ------------------------------------------------------------ harness


def fields_from_hex(h):
    def t(x):
        return bytes.fromhex(x).decode("utf-8")

    return {
        "id": t(h["id"]),
        "from": t(h["from"]),
        "to": t(h["to"]),
        "kind": t(h["kind"]),
        "correlation_id": None if h["correlation_id"] is None else t(h["correlation_id"]),
        "body": t(h["body"]),
        "artifact_refs": [t(r) for r in h["artifact_refs"]],
    }


def body_literal(wire):
    start = wire.index(b'"body":') + len(b'"body":')
    if wire[start:start + 1] == b" ":
        start += 1
    end = wire.index(b'"artifact_refs"')
    end = wire.rindex(b",", start, end)
    return wire[start:end]


def emit(record):
    print(json.dumps(record, ensure_ascii=True))


def main():
    corpus = json.load(open(sys.argv[1], encoding="utf-8"))
    for name, (encode, decode) in PROFILES.items():
        for v in corpus["vectors"]:
            wire = bytes.fromhex(v["wire_hex"])
            expected = fields_from_hex(v["fields_utf8_hex"]) if v["class"] == "positive" else None
            encode_result, encoded_hex = "n/a", None
            if expected is not None:
                try:
                    got = encode(expected)
                    encode_result = "match" if got == wire else "mismatch"
                    encoded_hex = None if got == wire else got.hex()
                except Exception as e:  # noqa: BLE001 - observation harness
                    encode_result, encoded_hex = "error", str(e)
            try:
                fields = decode(wire)
                decode_result, detail = "accept", ""
                fields_match = None if expected is None else fields == expected
            except Reject as e:
                decode_result, detail, fields_match = "reject", str(e), None
            if expected is not None:
                ok = encode_result == "match" and decode_result == "accept" and fields_match
            else:
                ok = decode_result == "reject"
            emit({"type": "vector", "impl": name, "vector": v["id"], "class": v["class"],
                  "encode": encode_result, "encoded_hex": encoded_hex, "decode": decode_result,
                  "fields_match": fields_match, "detail": detail, "pass": bool(ok)})
        golden = corpus["golden_fields"]
        for cp in corpus["escape_probe_codepoints"]:
            fields = dict(golden, kind="message", correlation_id=None, artifact_refs=[], body=chr(cp))
            try:
                emit({"type": "escape", "impl": name, "codepoint": cp,
                      "literal_hex": body_literal(encode(fields)).hex(), "error": None})
            except Exception as e:  # noqa: BLE001
                emit({"type": "escape", "impl": name, "codepoint": cp,
                      "literal_hex": None, "error": str(e)})


if __name__ == "__main__":
    main()
