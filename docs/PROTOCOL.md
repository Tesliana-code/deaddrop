# Protocol

**Status:** V0 semantic and wire contract frozen

## 1. Goals

The Deaddrop protocol supports:

- asynchronous peer messaging
- explicit sender and recipient identity
- stable message identity
- immutable artifact references
- provenance
- correlation across handoffs
- acknowledgments
- duplicate-safe processing
- version evolution
- transport independence
- authenticity and integrity through a dedicated cryptographic layer
- confidentiality through established cryptographic mechanisms where required

## 2. Envelope V0

The semantic envelope is:

```text
EnvelopeV0
├── protocol
├── id
├── from
├── to
├── kind
├── correlation_id?
├── body
└── artifact_refs[]
```

Its canonical V0 wire representation is UTF-8 JSON with this field order:

```json
{
  "protocol": "deaddrop/0",
  "id": "msg-wire-1",
  "from": "node-a:agent:deaddrop",
  "to": "node-b:agent:deaddrop",
  "kind": "handoff",
  "correlation_id": "corr-wire-1",
  "body": "continue this task",
  "artifact_refs": [
    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  ]
}
```

The example above is formatted for readability. The canonical wire bytes are compact and are defined normatively by rules R1–R7 below. The key words MUST, MUST NOT and SHOULD are normative.

These rules describe the behavior of the reference implementation (`deaddrop-protocol`) and do not change it. They are language-independent: implementations in other languages MUST reproduce the same bytes without relying on any particular JSON library's defaults.

### R1 Document form

The wire form is the UTF-8 bytes of exactly one JSON object. There is no byte order mark, no whitespace outside strings, and no bytes before or after the object.

### R2 Members

There are exactly eight members, with these literal names, in this fixed order:

```text
protocol, id, from, to, kind, correlation_id, body, artifact_refs
```

Members are separated by `,`. Each name is followed immediately by `:`. No member may be added, omitted, repeated or reordered.

### R3 Values

| member | value |
|---|---|
| `protocol` | exactly the string `deaddrop/0` |
| `id`, `from`, `to` | identifier string (R5) |
| `kind` | one string from the V0 vocabulary in §5 |
| `correlation_id` | identifier string (R5), or the literal `null` if and only if absent |
| `body` | string |
| `artifact_refs` | array of canonical `sha256:<64 lowercase hex>` strings (§7), in sender order, duplicates preserved; `[]` when empty, never `null`; elements separated by `,` |

No numbers, booleans or nested objects occur anywhere in the envelope.

### R4 String serialization

A string is a sequence of Unicode scalar values; surrogate code points cannot be represented. A string is serialized as `"`, then each scalar in order as follows, then `"`:

| scalar | bytes |
|---|---|
| U+0022 `"` | `\"` |
| U+005C `\` | `\\` |
| U+0008 | `\b` |
| U+0009 | `\t` |
| U+000A | `\n` |
| U+000C | `\f` |
| U+000D | `\r` |
| any other U+0000..U+001F | `\u00` followed by two **lowercase** hex digits |
| every other scalar | its UTF-8 encoding, unescaped |

"Every other scalar" includes `/`, `<`, `>`, `&`, `'`, U+007F, U+0080..U+009F, U+2028, U+2029, U+FEFF, noncharacters, and non-BMP scalars (4-byte UTF-8, never surrogate-pair escapes).

No Unicode normalization is applied; differently normalized spellings are different strings.

This matches the ECMAScript `JSON.stringify` / RFC 8785 string serialization for well-formed strings. Envelope V0 is **not** RFC 8785 as a whole: R2 fixes member order rather than sorting it. Implementations MUST NOT substitute RFC 8785, CBOR or any other canonicalization scheme.

### R5 Identifier profile

`id`, `from`, `to` and a present `correlation_id` MUST satisfy all of:

- non-empty
- the first and the last scalar are not Unicode `White_Space`: U+0009..U+000D, U+0020, U+0085, U+00A0, U+1680, U+2000..U+200A, U+2028, U+2029, U+202F, U+205F, U+3000
- no scalar in general category `Cc`: U+0000..U+001F, U+007F..U+009F

Nothing else is restricted. Interior spaces, non-ASCII, format characters (for example U+200B, U+202E) and unnormalized text are accepted by V0. Implementations MUST NOT apply stronger identifier restrictions or normalization to V0 envelopes.

### R6 Acceptance

A receiver accepts bytes `B` if and only if all of the following hold:

1. `B` is well-formed UTF-8.
2. `B` parses as JSON into a value satisfying R2, R3 and R5.
3. Re-encoding that value per R1–R4 yields bytes byte-for-byte equal to `B`.

Step 3 is a security and interoperability requirement, not a serializer preference. When `B` equals a canonical encoding, it is by construction free of duplicate members, alternative escapes, whitespace, BOMs and invalid UTF-8. Without step 3, ordinary JSON parsers can accept duplicate members and silently change `body` or `to` between the bytes that were received (and later signed or verified) and the value that is acted on.

### R7 Encoder obligation

An encoder MUST refuse, rather than emit, a value that violates R3 or R5, or that contains a surrogate code point. It MUST NOT emit nonconforming wire bytes.

### Implementer note

Measured against the reference implementation (Python 3.12, Go 1.27):

- Python `json.dumps` with defaults is not canonical (spaced separators, non-ASCII escaped). `separators=(",", ":"), ensure_ascii=False` matches R4 but still needs R5 validation and the R6 byte-equality check.
- Go `encoding/json` v1 escapes `<`, `>`, `&`, U+2028 and U+2029. `SetEscapeHTML(false)` alone is insufficient: U+2028/U+2029 remain escaped. A nil slice encodes as `null` instead of `[]`.
- Ordinary parsers accept variants V0 rejects: duplicate members (last-wins), BOMs, invalid UTF-8 (U+FFFD substitution), case-insensitive member names.

Therefore implementations MUST perform the R6 canonical re-encode byte-equality check, and SHOULD validate against the published conformance corpus, `protocol/vectors/envelope-v0.json` (29 positive, 60 negative vectors, exact bytes as hex).

## 3. Message identity

A message keeps the same `MessageId` across retransmission.

Exact replay is idempotent.

Reusing one `MessageId` for different envelope content produces an identity conflict at the persistence boundary.

## 4. Explicit addressing

Sender and recipient identity are part of the semantic envelope.

Recipient scope is carried directly by the message rather than inferred from transport location, channel membership, or relay behavior.

## 5. Message classes

V0 defines this stable vocabulary:

- `message`
- `request`
- `response`
- `handoff`
- `task_claim`
- `checkpoint`
- `acknowledgment`
- `error`
- `capability_declaration`

These classes describe coordination semantics.

Authorization remains an explicit and separate contract.

## 6. Correlation

Related messages may share a `CorrelationId`.

Correlation connects work across request/response, delegation, checkpoints, handoffs, and acknowledgments while preserving independent message identity.

## 7. Artifact references

Envelope V0 carries ordered immutable artifact references.

V0 artifact identity uses canonical SHA-256 references:

```text
sha256:<64 lowercase hexadecimal characters>
```

Artifact bytes remain independent from the message body and can move through separate storage or transport paths.

## 8. Delivery semantics

V0 is asynchronous and duplicate-tolerant.

The protocol is designed around:

- delay
- reconnect
- duplicate delivery
- sender restart
- receiver restart
- transport retry

Handlers use idempotent behavior wherever repeated processing could produce side effects.

Delivery evidence is append-only and stored separately from the immutable envelope.

```text
EnvelopeV0
    │
    ├── immutable message
    │
    └── delivery evidence
            │
            ▼
      rebuildable projection
```

A delivery projection is derived knowledge. The persisted envelope and delivery evidence remain the durable local record.

## 9. Acknowledgments

An acknowledgment names a specific protocol event.

Receipt, verification, recipient acknowledgment, task completion, and external side-effect success remain separate facts.

This keeps protocol evidence precise across retries and partial failure.

## 10. Canonical wire acceptance

A V0 implementation accepts a wire envelope exactly when the R6 acceptance rule in §2 holds: the bytes are valid UTF-8, parse into the frozen V0 schema and value constraints (R2, R3, R5), and re-encode per R1–R4 to the exact original bytes.

Canonical re-encode equality is a security and interoperability requirement, not a serializer preference.

## 11. Transport independence

The same canonical semantic envelope can move over different transports.

Transport owns delivery mechanics.

The envelope owns message meaning.

Relays, local IPC, peer links, mailbox-style services, and future transports can carry the same V0 bytes.

## 12. Persistence contract

A local node can durably preserve:

- the exact Envelope V0
- ordered artifact references
- append-only delivery evidence

After restart, the node can reload the exact envelope, reload the exact evidence history, and deterministically rebuild delivery projection state.

```text
canonical envelope
        │
        ├── durable message store
        │
        └── durable delivery evidence
                    │
                 restart
                    │
                    ▼
          exact local reconstruction
```

## 13. Cryptographic layer

Canonical Envelope V0 bytes are the input material for the next protocol layer.

That layer will define:

- domain-separated signing transcript
- signature algorithm suite
- public-key representation
- verification behavior
- key rotation
- encryption envelope
- versioned crypto identifiers

Deaddrop uses established, reviewable cryptographic primitives and libraries.

Cryptographic choices receive dedicated ADRs and independent review before production use.
