# Local Node and Storage

**Status:** Public architecture baseline  
**Note:** This document intentionally avoids freezing a database schema before implementation evidence exists.

## 1. Local ownership

The node is the operator's primary state boundary.

A central service may assist with delivery or synchronization but must not become the hidden owner of local identity, private history, or authorization.

## 2. Conceptual stores

A local node is expected to maintain durable state for concepts such as:

- identities
- peers
- messages
- tasks
- artifacts
- receipts
- capability grants
- provenance
- local policy
- synchronization cursors where needed

The physical schema may evolve.

## 3. Sandbox vs durable state

Temporary model/tool scratch state should not automatically become durable memory.

Durable memory should not automatically become coordination history.

Coordination history should not automatically become source authority.

These boundaries must be reflected in storage design.

## 4. Offline behavior

The local node should remain inspectable and useful when disconnected.

Operations requiring a remote peer or source can queue, fail clearly, or wait.

The UI should not pretend remote state is current when it is not.

## 5. Durable log semantics

Where coordination history matters, append-oriented records are preferred over silent mutation because they preserve how a state was reached.

Derived views may be rebuilt from durable records.

## 6. Secrets

Secrets and private keys require storage appropriate to the host operating system and threat model.

They must not be placed in ordinary application tables or logs for convenience.

## 7. Retention

Storage classes should have explicit retention policy.

An implementation should be able to explain why a datum remains stored.

## 8. Export and portability

Local ownership implies practical portability.

Where feasible, users should be able to export their own messages, peer records, artifacts, and provenance without requiring a central platform to interpret them.

## 9. Database choice

A small embedded database is a natural V0 candidate.

The exact database and schema will be selected by ADR after the first protocol and lifecycle requirements are frozen.
