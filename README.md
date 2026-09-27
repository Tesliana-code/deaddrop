# Deaddrop

> **The interface with the dead internet.**

The web grew around a human operating a browser.

Deaddrop begins with a different interface:

**humans express intent; software does the walking.**

```text
human
  ↓
intent
  ↓
local agent
  ↓
peers / services / sources
  ↓
evidence
  ↓
result or explicitly authorized action
```

**The interface is intent.**

---

## Why Deaddrop

The network is becoming increasingly machine-traversed.

Agents can query sources, cross systems, verify evidence, negotiate with services, coordinate with peers, and return with the thing a person actually asked for.

Deaddrop is an experiment in building that layer around a few strong ideas:

- intent over navigation
- task completion over attention capture
- recipient scope over ambient publication
- capability routing over popularity
- local ownership over platform dependency
- provenance over ambiguity
- explicit authority over implied authority
- inspectable protocol over hidden protocol behavior

**Deaddrop is an instrument.**

You use it to accomplish something, receive the result, and move on.

---

## The dead-drop primitive

A dead drop is recipient-scoped exchange.

Something is intentionally prepared for a specific recipient, delivered through a transport, verified, and acknowledged.

```text
sender
  ↓
signed message
  ↓
immutable artifact reference
  ↓
recipient
  ↓
verification
  ↓
acknowledgment
```

A message can point to an artifact by content identity:

```text
sha256:9f2c…
```

The hash gives both sides a stable reference to **the same exact object** across storage systems, filenames, interfaces, and transports.

The network optimizes for a successful handoff.

---

## Share messages. Keep sandboxes private.

A sandbox is a workshop.

Coordination happens through explicit exchange:

- messages
- artifact references
- provenance
- task claims
- acknowledgments
- checkpoints
- capability declarations

The operating rule is:

> **Share messages. Keep sandboxes private.**

And the distributed version:

> **Agents meet by reference across isolated runtimes.**

Private working state stays private. Handoffs stay inspectable. Different agents, models, machines, and runtimes cooperate through stable contracts.

---

## Local first

A Deaddrop node belongs to its operator.

Its local state remains useful and meaningful on its own.

The network extends the node.

```text
LOCAL NODE
├── identity
├── peers
├── messages
├── tasks
├── artifacts
└── local agent

             │
             │ explicit exchange
             ▼

NETWORK
├── trusted peers
├── services
├── relays
└── source systems
```

Transport, authority, identity, and ownership remain separate concepts.

This separation keeps the architecture portable across protocols, runtimes, and deployment models.

---

## Privacy is architectural

Deaddrop treats publication, indexing, sharing, storage, and authorization as distinct actions.

These are core laws:

> **Publicness is explicit.**

> **Indexing requires explicit scope and authority.**

> **Infrastructure learns the minimum required for its role.**

That principle shapes the system toward:

- local-first state
- recipient-scoped communication
- explicit sharing
- end-to-end confidentiality where applicable
- signed messages
- artifact integrity
- minimal intermediary knowledge
- provenance
- least privilege
- explicit authorization for consequential actions
- purpose-limited observability
- task-oriented interfaces

A relay carries messages with minimal knowledge.

An artifact store preserves bytes and integrity.

Discovery operates within explicit scope.

An agent receives the capabilities required for its task.

Privacy shapes the architecture from the beginning.

---

## Capability graph

An intent network asks:

```text
Who can do this?
Who is trusted for this domain?
Which source is authoritative?
Which artifact proves the claim?
Which peer can accept this handoff?
```

That is a **capability graph**.

Its purpose is routing, verification, delegation, and action.

---

## Authority stays explicit

Deaddrop separates kinds of state by role:

```text
LOCAL SANDBOX
private ephemeral working state

DURABLE MEMORY
long-lived retained knowledge

COORDINATION HISTORY
messages, ACKs, claims, handoffs, checkpoints

SOURCE AUTHORITY
the repository / database / document / service
that owns the fact
```

Each domain names its source of authority.

Messages carry claims and evidence.

Memory carries retained knowledge.

Projections carry derived views.

Coordination history carries what participants exchanged.

Source systems retain ownership of the facts they govern.

This distinction becomes especially important when agents cooperate across machines and runtimes.

---

## Human authority is part of the architecture

Agents can search, compare, summarize, prepare, route, verify, and coordinate.

Consequential actions cross an explicit authorization boundary.

Examples include:

- purchases
- publication
- account changes
- destructive operations
- external commitments
- privilege changes

The person retains the final word where human authorization is required.

Automation remains powerful because authority remains legible.

---

## Public architecture

This repository is the **open, inspectable substrate** of Deaddrop.

Public work includes:

- local-first client architecture
- peer and identity models
- message envelopes
- signing and encryption interfaces
- immutable artifact references
- provenance
- authorization boundaries
- privacy-preserving transports
- safe interoperability
- reference implementations
- protocol tests and test vectors
- threat modeling
- accessibility
- offline behavior

Security-critical foundations stay inspectable.

Cryptographic choices stay reviewable.

Protocol invariants stay testable.

Interoperability stays documented.

### Defensive operations

Operational defense follows a separate need-to-know boundary.

The public project documents security guarantees, threat classes, trust assumptions, protocol constraints, and interoperability behavior.

Production defense keeps deployment-specific heuristics, thresholds, quarantine triggers, anomaly scoring, defensive routing, decoys, incident playbooks, and live countermeasures within their operational scope.

In short:

> **Open the protocol. Keep defensive operations need-to-know.**

---

## First milestone

The first useful Deaddrop is intentionally small.

Two nodes are enough.

```text
NODE A
  ↓
creates local identity
  ↓
trusts NODE B
  ↓
sends a signed/encrypted message
  ↓
references an immutable artifact
  ↓
NODE B receives asynchronously
  ↓
verifies
  ↓
ACKs
```

A local agent uses the same mechanism through a small interface.

That gives us the heart of the system before transport, discovery, and federation expand.

---

## Project shape

The implementation grows around a small local core:

```text
Deaddrop
├── local shell
├── identity
├── peers
├── messages
├── tasks
├── artifacts
├── transport
└── local agent interface
```

We favor boring, inspectable primitives.

The desktop client should feel like an instrument:

**small, fast, quiet, task-shaped.**

You open it because you intend to do something.

You leave when the task is complete.

---

## Contributing

Deaddrop is public so the protocol, privacy model, and interoperability layer can survive serious external review.

Useful contributions include:

- local-first storage
- native desktop work
- cryptographic review
- protocol design
- message interoperability
- artifact integrity
- provenance
- safe peer UX
- offline behavior
- test vectors
- accessibility
- threat modeling
- documentation

Project quality is measured through:

- task completion
- correctness
- integrity
- privacy
- interoperability
- inspectability
- reliability
- accessibility
- efficient handoff
- clear authority

A fuller contribution guide will land before implementation work opens broadly.

---

## Status

**Very early.**

Right now the architecture matters more than the feature count.

We are defining primitives, trust boundaries, and invariants first, then expanding implementation around them.

That is intentional.

---

## One sentence

> **Deaddrop is a local-first network shell where humans express intent, agents coordinate by message and immutable reference, and infrastructure learns the minimum required to complete the handoff.**

---

## Origin

Deaddrop grew out of a conversation about what happens when humans let agents traverse the network on their behalf.

The longer version became the essay **“Deaddrop.”**[^1]

[^1]: Ivana Vrtaric, *Deaddrop*, Medium, 2026 — https://medium.com/@ivavrtaric/deaddrop-fa09f97dba93

## Architecture documentation

See **[docs/README.md](docs/README.md)** for the complete public architecture map.
