# Deaddrop

> **The interface with the dead internet.**

The web was built around a human operating a browser.

Open a page.  
Find the menu.  
Accept the cookies.  
Close the popup.  
Search.  
Open five tabs.  
Compare.  
Fill the form.  
Prove you are human.  
Do it again tomorrow.

Deaddrop starts from a different assumption:

**humans should express intent; software should do the walking.**

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

The page is no longer the interface.

**The interface is intent.**

---

## Why Deaddrop

The internet is not dead because there are bots on it.

It becomes interestingly "dead" when humans no longer need to manually traverse interfaces designed around advertising, retention, ranking, tracking, SEO and engagement.

The network can remain very alive underneath.

Agents can query it, cross it, verify it, negotiate with it and return with the thing a person actually asked for.

Deaddrop is an experiment in building that layer without rebuilding the worst parts of the web.

No feed.

No followers.

No trending page.

No engagement score.

No recommendation treadmill.

No reason to make a person stare at the product longer than necessary.

**Deaddrop is an instrument, not a destination.**

---

## The dead-drop primitive

A dead drop is not a broadcast.

It is something intentionally left for a recipient.

That turns out to be a useful primitive for an agent-native network.

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

The hash is not decoration. It means both sides can refer to **the same exact object** without treating a mutable location, filename or UI page as truth.

The network should optimize for a successful handoff, not for attention.

---

## Share messages, not sandboxes

Agents do not need to live inside one giant shared runtime.

A sandbox is a workshop, not a communal kitchen.

Deaddrop treats coordination as explicit exchange:

- messages
- artifact references
- provenance
- task claims
- acknowledgments
- checkpoints
- capability declarations

The useful rule is:

> **Share messages, not sandboxes.**

And its stronger version:

> **Agents should meet by reference, not by cohabitation.**

This keeps private working state private, makes handoffs inspectable, and allows different agents, models, machines and runtimes to cooperate without pretending they are one process.

---

## Local first

A Deaddrop node belongs to its operator.

Its local state should remain useful without a central platform.

The network extends the node. It does not own it.

A rough mental model:

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

The long-term architecture may use several transports. The important part is that transport is not confused with authority, identity or ownership.

---

## Surveillance is not the business model

Deaddrop rejects surveillance-by-default as an architectural assumption.

Creating something does not mean publishing it.

Persisting something does not mean indexing it.

Sending something to one recipient does not mean exposing it to a hidden audience.

These are core laws:

> **Nothing is public merely because it exists.**

> **Existence is not consent to index.**

And for infrastructure:

> **The infrastructure should know less than the participants.**

That principle pushes the design toward:

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
- no behavioral telemetry as a product requirement
- no engagement graph disguised as infrastructure

A relay should not need message plaintext.

An artifact store should not need to know what an artifact means.

Discovery should not require reading private conversations.

An agent should not receive capabilities it does not need.

Privacy is not a settings page. It is a constraint on the shape of the system.

---

## Capability graph, not popularity graph

The social web asks:

```text
Who has the most followers?
What is trending?
What keeps people engaged?
```

An intent network should ask:

```text
Who can do this?
Who is trusted for this domain?
Which source is authoritative?
Which artifact proves the claim?
Which peer can accept this handoff?
```

That is a **capability graph**, not a popularity graph.

The goal is routing, verification and action.

Not influence.

---

## Authority stays explicit

An agent saying something does not make it true.

A cached result does not become a source of truth because it is convenient.

A coordination message does not become authority merely because it was signed.

Deaddrop keeps several kinds of state conceptually separate:

```text
LOCAL SANDBOX
private ephemeral working state

DURABLE MEMORY
long-lived knowledge

COORDINATION HISTORY
messages, ACKs, claims, handoffs, checkpoints

SOURCE AUTHORITY
the repository / database / document / service
that actually owns the fact
```

This distinction matters especially when agents cooperate across machines and runtimes.

---

## Human authority is part of the architecture

Agents can search, compare, summarize, prepare, route and coordinate.

That does not mean every possible action should become autonomous.

Consequential mutations should cross an explicit authorization boundary.

Examples include:

- purchases
- publication
- account changes
- destructive operations
- external commitments
- privilege changes

The person keeps the final word.

Automation is useful precisely because authority remains legible.

---

## What this public repository is for

This repository is the **open, inspectable substrate** of Deaddrop.

Appropriate public work includes:

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

Security-critical foundations should be inspectable.

Cryptography should not depend on obscurity.

Protocol invariants should be reviewable.

Interoperability should be documented.

### What is deliberately not documented here

Operational defensive mechanisms are a separate concern and are intentionally kept out of the public repository.

The public project can state its security guarantees, threat model and protocol assumptions without publishing the exact detection heuristics, thresholds, quarantine logic, incident-response mechanics or other defensive operational details used to protect deployed systems.

In short:

> **Open the protocol. Keep defensive operations need-to-know.**

---

## What Deaddrop is not

Deaddrop is not:

- another social network
- an AI feed
- a creator platform
- a follower graph
- an ad network
- a recommendation engine
- an SEO surface
- a surveillance product
- a shared mega-sandbox for autonomous agents
- a justification for removing humans from consequential decisions

If the project ever starts measuring success by time-on-app, something has gone wrong.

---

## First milestone

The first useful Deaddrop does not need to reinvent the entire internet.

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

And a local agent should be able to use that mechanism through a very small interface.

That gives us the heart of the system before we add more elaborate transport, discovery or federation.

---

## Project shape

The implementation is expected to grow around a small local core:

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

We favor boring, inspectable primitives over magical infrastructure.

The desktop client should feel like an instrument.

Small. Fast. Quiet.

You open it because you intend to do something.

Then you leave.

---

## Contributing

Deaddrop is public because the protocol, privacy model and interoperability layer should survive contact with people who did not design them.

Useful contributions will eventually include:

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

Things we do **not** want to optimize for:

- engagement
- retention
- virality
- follower growth
- popularity ranking
- behavioral advertising
- hidden telemetry
- dark patterns
- unsolicited bulk messaging
- attention capture

A fuller contribution guide will land before implementation work opens broadly.

---

## Status

**Very early.**

Right now the architecture matters more than the feature count.

We are defining the primitives, trust boundaries and invariants before making the system large enough to become confused about what it is.

That is intentional.

---

## One sentence

> **Deaddrop is a local-first network shell where humans express intent, agents coordinate by message and immutable reference, and the infrastructure is designed to know less than the participants.**

---

## Origin

Deaddrop grew out of a conversation about what happens when humans stop browsing the web like it is 1996 and let agents traverse the network on their behalf.

The longer version became the essay **“Deaddrop.”**[^1]

[^1]: Ivana Vrtaric, *Deaddrop*, Medium, 2026 — https://medium.com/@ivavrtaric/deaddrop-fa09f97dba93


## Architecture documentation

See **[docs/README.md](docs/README.md)** for the complete public architecture map.
