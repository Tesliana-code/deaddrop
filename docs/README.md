# Deaddrop Documentation

This directory is the public architectural specification for Deaddrop.

Read it in this order:

1. [CONSTITUTION.md](CONSTITUTION.md) — non-negotiable architectural invariants
2. [ARCHITECTURE.md](ARCHITECTURE.md) — system shape and component boundaries
3. [THREAT_MODEL.md](THREAT_MODEL.md) — public threat classes and trust assumptions
4. [PRIVACY.md](PRIVACY.md) — data scope, indexing, retention, telemetry, model access
5. [IDENTITY_AND_TRUST.md](IDENTITY_AND_TRUST.md) — node identity, trust, revocation, separation from authorization
6. [AUTHORIZATION.md](AUTHORIZATION.md) — read / prepare / propose / authorize / act / verify
7. [PROTOCOL.md](PROTOCOL.md) — public wire contract and delivery properties
8. [MESSAGING.md](MESSAGING.md) — dead-drop semantics, ACKs, handoffs, correlation
9. [ARTIFACTS.md](ARTIFACTS.md) — immutable content identity and provenance
10. [TRANSPORT_AND_RELAY.md](TRANSPORT_AND_RELAY.md) — mailbox transport without platform centrality
11. [LOCAL_NODE_AND_STORAGE.md](LOCAL_NODE_AND_STORAGE.md) — operator-owned local state
12. [CAPABILITY_ROUTING.md](CAPABILITY_ROUTING.md) — capability graph instead of popularity graph
13. [INTEROPERABILITY.md](INTEROPERABILITY.md) — multi-client, multi-model, multi-runtime compatibility
14. [AGENT_WIRE.md](AGENT_WIRE.md) — public Agent Wire contribution boundary inside Deaddrop
15. [adr/README.md](adr/README.md) — architecture decision record policy

Repository-level policies:

- [../CONTRIBUTING.md](../CONTRIBUTING.md)
- [../SECURITY.md](../SECURITY.md)

## Public / private line

The public repository explains:

- what the system guarantees
- what the system trusts
- how participants interoperate
- where authority lives
- how privacy is preserved by architecture
- which security properties are expected

The private defensive layer may contain deployment-specific operational defenses whose disclosure would materially weaken them.

> **Open the protocol. Keep defensive operations need-to-know.**

## Documentation authority

When documents disagree, use this order:

```text
CONSTITUTION
  ↓
accepted ADRs
  ↓
domain architecture documents
  ↓
implementation
```

Implementation convenience does not override a constitutional invariant.
