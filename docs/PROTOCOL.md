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

The canonical encoder emits compact UTF-8 JSON. The example above is formatted for readability.

The canonical byte contract fixes:

- field names
- field order
- UTF-8 JSON encoding
- explicit `null` for an absent `correlation_id`
- artifact reference order
- protocol identifier `deaddrop/0`
- message-kind vocabulary

The decoder reconstructs the semantic envelope and re-encodes it. Acceptance requires byte-for-byte equality with the canonical representation.

This gives one semantic Envelope V0 one canonical wire representation.

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

A V0 implementation accepts a wire envelope when:

- the bytes parse as the frozen V0 schema
- the protocol identifier is `deaddrop/0`
- identifiers satisfy their protocol constraints
- the message kind belongs to the V0 vocabulary
- artifact references parse canonically
- the schema contains the frozen V0 fields
- re-encoding produces the exact original bytes

This rule turns canonical encoding into an interoperability contract rather than a serializer preference.

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
