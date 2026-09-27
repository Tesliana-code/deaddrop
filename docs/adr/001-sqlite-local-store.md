# ADR-001: SQLite as the first durable local store

**Status:** Accepted  
**Date:** 2026-09-27

## Context

Deaddrop now has a proven persistence contract for append-only delivery evidence:

```text
immutable protocol evidence
        ↓
append-only persistence
        ↓
replay
        ↓
rebuildable derived projection
```

The storage boundary has already been defined independently of any database implementation.

The in-memory reference store proves the required semantics:

- exact replay is idempotent
- conflicting reuse of an event identity fails closed
- evidence remains scoped to its message
- distinct historical observations remain distinct
- storage does not derive delivery meaning
- core projections can be rebuilt from persisted evidence
- persistence and projection remain separate authorities

The next implementation step requires durable local persistence.

Deaddrop is local-first, so the first durable store should not require a network service or central database.

## Decision

Use **SQLite** as the first durable local persistence backend for Deaddrop nodes.

SQLite is an implementation of the existing storage contract.

It does not become source authority for domains owned elsewhere.

The initial SQLite backend will preserve append-only delivery evidence and must remain semantically compatible with the in-memory reference store.

## Why SQLite

SQLite fits the V0 requirements:

- embedded
- local-first
- no daemon required
- transactional
- mature
- portable
- inspectable
- easy to back up
- suitable for small desktop/native applications
- supports uniqueness constraints required for immutable event identity
- supports deterministic local persistence ordering without making that order protocol authority

It also keeps the first node deployable as a single local application rather than introducing infrastructure merely to store local state.

## Required invariants

### Event identity is immutable

```text
same ID + same evidence
→ AlreadyPresent

same ID + different evidence
→ identity conflict
→ FAIL CLOSED
```

### Persistence is append-only

Existing evidence must not be silently rewritten to represent a different event.

### Storage does not derive semantic status

SQLite may persist delivery facts, but it must not become authoritative for conclusions such as `Delivered`, `Successful`, `Completed`, or `CurrentStatus`.

Those belong to rebuildable core projections.

### Message boundaries remain explicit

Loading evidence for one message must not include evidence from another message.

### Replay remains safe

A projection rebuilt repeatedly from the same durable evidence must produce the same result.

### Local ordering is not protocol ordering

SQLite may retain local insertion order for deterministic reads and diagnostics.

That ordering must not be interpreted as global causal or protocol ordering unless a future protocol decision explicitly defines such semantics.

## Library choice

The Rust SQLite library is an implementation detail subordinate to this ADR.

The first implementation should prefer a small synchronous interface because the current `DeliveryEventStore` contract is synchronous and local.

Introducing async database semantics is not justified until the surrounding architecture requires it.

## Alternatives considered

### PostgreSQL

Rejected for local V0 because it introduces an external service, deployment requirements, credentials, network configuration, and operational ownership inconsistent with the first local-first node.

It may still be appropriate for future server or relay infrastructure.

### Flat files

Rejected as the primary durable store because they would require rebuilding transactional behavior, uniqueness guarantees, crash handling, indexing, and schema evolution.

### Custom append-only log

Deferred.

A custom log may eventually be useful for specialized replication or protocol work, but building one before proving the product would create unnecessary storage infrastructure.

### In-memory only

Retained as the semantic reference implementation, but insufficient for durable local operation.

## Consequences

### Positive

- Deaddrop becomes genuinely durable without introducing a server.
- Local state remains operator-owned.
- Storage semantics remain testable against the in-memory reference implementation.
- Unique event identity can be enforced at the database boundary.
- Schema migration can be explicit.

### Costs

- SQLite schema and migrations become maintained project artifacts.
- Corruption and migration failures need explicit handling.
- Concurrent access semantics must eventually be defined.
- Backup and deletion behavior must be documented before production use.

## Constitutional impact

This decision supports:

- **Local state belongs to the operator**
- **Infrastructure should know less than participants**
- **Authority is explicit**
- **Provenance travels with information**
- **No hidden centrality**

SQLite is local persistence infrastructure.

It must not become semantic authority merely because it stores durable evidence.

## Future reconsideration

This ADR does not require SQLite for every Deaddrop component.

Relays, multi-user services, large artifact stores, or federated infrastructure may use different persistence systems.

The public storage contract should allow those implementations without changing protocol truth.
