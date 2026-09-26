# Privacy Model

**Status:** Public architectural contract

Privacy in Deaddrop is not a preference layer added after implementation.

It shapes storage, transport, identity, logging, indexing, discovery, model access, authorization, retention, and failure behavior.

## 1. Governing laws

> **Nothing is public merely because it exists.**

> **Existence is not consent to index.**

> **The infrastructure should know less than the participants.**

## 2. Data states are distinct

Deaddrop must distinguish at least:

- local-only
- addressed to a specific recipient
- shared with a defined group
- deliberately public
- indexed / discoverable

These states are not interchangeable.

Moving between them requires explicit authority.

## 3. Recipient scope

A message is scoped to its intended recipient or audience.

Infrastructure must not silently enlarge that audience.

Replication required for delivery does not imply permission to inspect, index, profile, summarize, train on, or redistribute content.

## 4. Indexing

Indexing is treated as a separate action because it changes discoverability and therefore changes privacy.

An index must have:

- defined scope
- defined authority
- defined purpose
- defined retention
- defined audience

Ambiguity defaults to no indexing.

## 5. Intermediaries

Intermediaries receive only what they need.

Examples:

- relay: enough information to route/store opaque payloads
- artifact store: enough information to serve requested bytes
- discovery: enough information to resolve permitted capabilities
- local agent: only the context and capabilities needed for its task

## 6. Telemetry

Reliability telemetry may be necessary.

It must be:

- purpose-limited
- minimized
- documented
- separable from behavioral profiling
- retained only as long as justified

Deaddrop must not require attention tracking, engagement scoring, advertising identifiers, or cross-context behavioral profiles.

## 7. Model access

A model or agent does not receive content merely because the content exists locally.

Model access is an explicit processing boundary.

Implementations should make it possible to reason about:

- what content is sent to a model
- which model receives it
- whether the model is local or remote
- what capabilities are attached to the model invocation
- what result is retained

## 8. Retention

Retention must be intentional.

Different categories may require different lifetimes:

- ephemeral sandbox state
- durable memory
- coordination history
- artifacts
- receipts
- local logs

The existence of storage capacity is not a retention policy.

## 9. Deletion

Deletion semantics must be explicit about scope.

Deleting a local record cannot truthfully promise deletion from a peer that already received it.

Deleting a mutable alias does not erase an immutable artifact already distributed.

The UI and protocol must not imply stronger guarantees than the system can provide.

## 10. Failure behavior

When privacy scope is unclear, failure should be conservative.

The system may refuse, defer, or request clarification rather than silently widening access.

## 11. Public/private security boundary

Privacy guarantees and protocol behavior are public.

Exact operational mechanisms used to detect attempts to defeat those guarantees are not required to be public.
