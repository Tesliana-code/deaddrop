# Deaddrop Architecture

**Status:** Public architecture baseline  
**Authority:** This document describes system shape. The Constitution defines the invariants it must preserve.

## 1. Purpose

Deaddrop is a local-first network shell for humans and agents.

Its job is to turn intent into verifiable network work without requiring a person to manually traverse browser-era interfaces or surrender private state to a central platform.

```text
HUMAN
  ↓
INTENT
  ↓
LOCAL SHELL
  ↓
LOCAL AGENT
  ↓
COORDINATION / TRANSPORT
  ↓
PEERS / SERVICES / SOURCES
  ↓
EVIDENCE
  ↓
RESULT OR EXPLICITLY AUTHORIZED ACTION
```

The page is not the primary interaction primitive.

The primary primitive is intent.

## 2. The local node

A Deaddrop node is the operator-owned unit of execution.

Conceptually:

```text
LOCAL NODE
├── identity
├── peer book
├── messages
├── tasks
├── artifact index
├── receipts / acknowledgments
├── capability records
├── local agent interface
└── local policy / authorization state
```

The exact implementation may evolve. The ownership boundary must not.

The network extends the node. It does not own the node.

## 3. Four state domains

Deaddrop keeps these domains separate:

```text
LOCAL SANDBOX
ephemeral private working state

DURABLE MEMORY
retained knowledge useful across runs

COORDINATION HISTORY
messages, claims, ACKs, handoffs, checkpoints

SOURCE AUTHORITY
the repository / database / document / service
that actually owns a fact
```

A coordination message is evidence of communication, not automatic source authority.

An agent memory is retained context, not automatic source authority.

A cache is an optimization, not automatic source authority.

## 4. Coordination boundary

Agents coordinate through explicit exchange rather than shared mutable runtime state.

Allowed coordination primitives include:

- messages
- immutable artifact references
- task claims
- acknowledgments
- checkpoints
- provenance
- capability declarations
- authorized requests

The architectural rule is:

> **Share messages, not sandboxes.**

## 5. Components

### Local shell

Human-facing interface for expressing intent, inspecting evidence, authorizing consequential actions, and reviewing results.

It should behave like an instrument, not a destination.

### Local agent interface

A narrow interface through which an authorized local agent can query local state, persist durable outputs, and coordinate with peers.

The interface must not silently grant broad authority merely because an agent is local.

### Peer

A separately identified node or agent context capable of receiving messages or participating in a task.

### Relay

An optional store-and-forward mailbox for asynchronous delivery.

A relay is transport infrastructure, not the owner of identity, private state, or social relationships.

### Artifact store

Storage capable of serving bytes referenced by immutable content identity.

The artifact store does not become authoritative about the meaning of those bytes.

### Source system

A system that owns some external fact or mutable state: a repository, database, SaaS service, document store, device, account, or similar authority.

## 6. Data flow

A typical flow:

```text
human intent
  ↓
local interpretation
  ↓
local policy check
  ↓
query / message / service call
  ↓
source evidence or peer response
  ↓
provenance + artifact refs
  ↓
local reasoning
  ↓
result
  ↓
authorization boundary if mutation is consequential
  ↓
external action
  ↓
receipt / updated source evidence
```

## 7. Replaceable layers

The architecture must permit replacement of:

- desktop UI
- agent/model
- local database
- relay implementation
- transport
- artifact transport
- external service adapters

without redefining:

- operator ownership
- recipient scope
- authority
- provenance
- artifact integrity
- authorization boundaries
- privacy invariants

## 8. Trust boundaries

The design assumes that nodes, relays, peers, agents, dependencies, and endpoints can fail independently.

No component should receive more information or authority than its role requires.

See [THREAT_MODEL.md](THREAT_MODEL.md) and [AUTHORIZATION.md](AUTHORIZATION.md).

## 9. Public vs defensive architecture

The public repository documents the protocol, threat model, trust boundaries, cryptographic interfaces, and security invariants.

Operational defense remains separate.

> **Open the protocol. Keep defensive operations need-to-know.**

## 10. V0 success condition

The first architecture milestone is deliberately small:

```text
NODE A
  ↓
establishes identity
  ↓
trusts NODE B
  ↓
sends signed + confidential message
  ↓
references immutable artifact
  ↓
NODE B receives asynchronously
  ↓
verifies
  ↓
ACKs
```

A local agent must be able to perform the same flow through a narrow local interface.

That is enough to prove the core before discovery, federation, or larger coordination layers are added.
