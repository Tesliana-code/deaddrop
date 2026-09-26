# Agent Wire inside Deaddrop

**Status:** Public relationship and contribution boundary

Agent Wire is the agent-coordination research and implementation lane inside the broader Deaddrop architecture.

Deaddrop defines the human-facing network shell, trust boundaries, protocol invariants, privacy model, and local-first ownership model.

Agent Wire explores how agents cooperate inside those constraints.

## 1. Core principle

> **Share messages, not sandboxes.**

Agent Wire should make cross-agent work explicit and inspectable rather than depending on shared mutable scratch space.

## 2. Minimal agent-facing API

The working conceptual API remains deliberately small:

### `wire_query`

Read or search permitted memory, coordination history, artifact metadata, provenance, task state, and other authorized local context.

### `wire_exec`

Persist durable knowledge or artifacts through explicit storage boundaries.

### `wire_coordinate`

Send messages, delegate, claim work, checkpoint, acknowledge, hand off, release, or otherwise coordinate with peers.

These names describe capability classes. The concrete public API is not frozen yet.

## 3. Agent identity

A working logical identity is:

```text
host:harness:project
```

This gives coordination a stable referent across runs without pretending every agent shares one process.

## 4. Coordination record

Agent Wire favors explicit records such as:

- task delegation
- claim
- checkpoint
- handoff
- acknowledgment
- artifact publication
- provenance reference
- completion
- release

Coordination history should be durable where reconstruction matters.

## 5. Source authority

Agent Wire memory must not become a competing source of truth.

An agent can remember that a repository had a particular commit.

The repository remains authoritative about its current commit.

An agent can remember a document excerpt.

The document remains authoritative about its contents.

## 6. Contribution areas

Agent Wire contributors can work publicly on:

- message/handoff semantics
- correlation IDs
- task lifecycle
- artifact references
- provenance
- idempotency
- replay-safe coordination
- multi-runtime interoperability
- local durable coordination history
- narrow agent capability boundaries
- conformance tests

## 7. Boundaries

Agent Wire must inherit the Deaddrop Constitution.

It must not introduce:

- shared mega-sandbox assumptions
- hidden audience expansion
- ambient broad capabilities
- source-authority confusion
- engagement incentives
- surveillance as coordination infrastructure

Operational defense remains outside the public repository.

Experimental machine-only protocol research may be incubated separately until its semantics and abuse properties are understood.

## 8. Why this belongs here

The same problem exists at both scales:

Humans should not have to manually walk every interface.

Agents should not have to share one unhygienic mutable environment just to cooperate.

Both are solved by clearer intent, explicit messages, stable references, provenance, bounded authority, and inspectable handoffs.
