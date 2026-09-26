# Protocol

**Status:** Pre-implementation public contract  
**Note:** Field names below are illustrative until a versioned wire schema is frozen.

## 1. Goals

The Deaddrop protocol should support:

- asynchronous peer messaging
- explicit sender and recipient identity
- message authenticity
- content confidentiality where required
- immutable artifact references
- provenance
- correlation across handoffs
- acknowledgments
- duplicate-safe processing
- version evolution
- transport independence

## 2. Envelope

Illustrative shape:

```json
{
  "protocol": "deaddrop/0",
  "id": "msg_...",
  "from": "host:harness:project",
  "to": "host:harness:project",
  "kind": "handoff",
  "subject": "Example",
  "body": "Result and context",
  "artifact_refs": ["sha256:..."],
  "correlation_id": "corr_...",
  "created_at": "...",
  "signature": "..."
}
```

A frozen V0 schema will be defined before interoperability claims are made.

## 3. Required protocol properties

### Stable message identity

A message has an identifier that does not change when retransmitted.

### Explicit addressing

Recipient scope is encoded, not inferred from location or channel membership.

### Authenticity

Recipients can determine which cryptographic identity signed a message.

### Integrity

Tampering must be detectable.

### Confidentiality

Messages intended to be private must be protected by established cryptographic mechanisms rather than custom encryption.

### Correlation

Related messages may share a correlation identifier without requiring shared process state.

### Artifact references

Large or durable evidence should be referenceable independently of the message body.

### Versioning

Protocol version must be explicit.

## 4. Message classes

Initial semantic classes may include:

- message
- request
- response
- handoff
- task_claim
- checkpoint
- acknowledgment
- error
- capability_declaration

These are coordination semantics, not authorization grants by themselves.

## 5. Delivery semantics

V0 should assume asynchronous delivery.

The protocol must tolerate:

- delay
- reconnect
- duplicate delivery
- sender restart
- receiver restart
- transport retry

Exactly-once network delivery is not assumed.

Handlers should therefore be idempotent where side effects are possible.

## 6. Acknowledgments

An ACK confirms a defined protocol event such as receipt or accepted processing.

It must not be overloaded to mean that the recipient agrees with the content or that an external action succeeded.

Those are separate facts.

## 7. Errors

Errors should be explicit enough for interoperability without leaking unnecessary defensive detail.

Protocol errors must not become an oracle exposing private operational controls.

## 8. Transport independence

The same semantic envelope should be able to move over multiple transports.

Transport is responsible for delivery.

The envelope is responsible for meaning.

## 9. Security

No custom cryptographic primitive should be invented for Deaddrop.

Algorithm suites, key formats, rotation, and encryption details require dedicated ADRs and independent review before production use.
