# Transport and Relay

**Status:** Public architecture baseline

## 1. Transport is replaceable

Transport moves protocol objects.

It does not define their meaning, authority, identity, or ownership.

Deaddrop should be able to evolve from a simple relay to additional direct or federated transports without rewriting its trust model.

## 2. Why V0 uses a relay concept

Real networks include:

- NAT
- firewalls
- sleeping laptops
- intermittent connectivity
- mobile clients
- peers that are not online at the same time

A minimal store-and-forward relay solves asynchronous delivery without requiring global peer-to-peer networking in the first implementation.

## 3. Relay responsibility

A relay may need to:

- accept an opaque addressed payload
- retain it for a bounded period
- make it available to the intended recipient
- support delivery acknowledgment
- enforce basic protocol limits

It should not require message plaintext.

## 4. What a relay must not become

A relay must not quietly become:

- identity authority
- source of truth for private node state
- public content index
- owner of the peer graph
- behavioral analytics platform
- engagement ranking service
- advertising layer
- recommendation engine

> **A mailbox is not a platform.**

## 5. Metadata minimization

Some routing metadata may be unavoidable.

The architecture should minimize:

- unnecessary sender/recipient exposure
- long retention of delivery metadata
- cross-context identifiers
- globally queryable relationship graphs

Exact production defensive telemetry is outside the public repository.

## 6. Failure semantics

Clients must tolerate:

- relay unavailable
- delayed delivery
- duplicate delivery
- retry
- partial outage
- message expiration according to explicit policy

Relay failure must not corrupt local source authority.

## 7. Future transports

Possible later transports include:

- direct peer connection
- multiple independent relays
- federated relays
- content-addressed artifact transfer
- local-network transport

No future transport may silently weaken Constitution guarantees.

## 8. Federation

Federation is not a goal by itself.

It is useful only if it reduces dependency while preserving recipient scope, provenance, integrity, and understandable trust boundaries.
