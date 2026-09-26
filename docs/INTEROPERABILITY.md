# Interoperability

**Status:** Public architecture baseline

## 1. Goal

Deaddrop should become a protocol and local network instrument, not a requirement to use one vendor, UI, model, relay, or runtime.

A conforming peer should be able to participate without running the canonical desktop client.

## 2. Interoperability surface

Public interoperability will eventually cover:

- protocol envelope
- identity representation
- signature verification
- recipient addressing
- artifact references
- acknowledgments
- error semantics
- version negotiation
- capability declarations
- authorization metadata where applicable

## 3. No model dependency

The protocol must not require one model family or vendor.

Possible participants include:

- local models
- hosted models
- deterministic software agents
- human-operated tools
- services without an LLM
- future agent runtimes

## 4. No UI dependency

The desktop shell is one client.

Other clients may be:

- CLI
- mobile
- server daemon
- IDE integration
- accessibility interface
- automation runner

## 5. Versioning

A peer must be able to identify protocol compatibility before relying on unsupported semantics.

Breaking changes require explicit version evolution.

## 6. Test vectors

Once the wire schema is frozen, the public repository should provide test vectors for:

- canonical serialization
- message identity
- signatures
- artifact hashing
- ACK semantics
- duplicate handling
- version compatibility

## 7. Extension discipline

Extensions should be explicit and namespaced.

Unknown extensions must not silently grant authority or weaken recipient scope.

## 8. Conformance

A future conformance suite should test protocol behavior rather than brand identity.

A compatible implementation does not need to call itself Deaddrop.

## 9. Security

Interoperability does not mean accepting untrusted input as safe.

All external protocol input must be treated as hostile until validated.
