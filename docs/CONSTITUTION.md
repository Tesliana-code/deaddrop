# Deaddrop Constitution

**Status:** Foundational architectural invariant  
**Applies to:** protocol, clients, relays, services, storage, agents, integrations, tooling, and governance

Deaddrop is not defined by a particular UI, transport, database, model, runtime, or implementation language.

Those may change.

This document defines the constraints that must not quietly change with them.

A contribution may be technically elegant, useful, popular, or commercially attractive and still be incompatible with Deaddrop if it violates these principles.

---

## Article I — Existence is not publication

> **Nothing is public merely because it exists.**

Creating information is not the same as publishing it.

Persisting information is not the same as indexing it.

Making information technically reachable is not the same as granting permission to inspect, classify, summarize, profile, train on, redistribute, or expose it.

No Deaddrop component may treat mere existence as evidence of public intent.

Publicness must be explicit.

---

## Article II — Existence is not consent to index

> **Existence is not consent to index.**

Indexing is an action with consequences.

It creates discoverability.

It changes audience.

It changes the risk profile of information.

It can enable correlation, profiling, inference, aggregation, ranking, recommendation, surveillance, or reuse far beyond the context in which the information was created.

Therefore Deaddrop must not silently convert reachable information into searchable information.

A component that indexes data must have a clear scope, authority, purpose, retention model, and audience.

If those are ambiguous, the safe default is not to index.

---

## Article III — Infrastructure should know less than participants

> **The infrastructure should know less than the participants.**

Intermediaries should possess the minimum knowledge required to perform their function.

A relay should not need message plaintext.

An artifact store should not need artifact semantics.

Discovery should not require reading private conversations.

A routing component should not receive unrelated private state.

A service should not infer identity, relationships, behavior, or intent merely because the data required to do so happens to pass through it.

The architecture should minimize the amount of information available to intermediaries rather than merely promising not to misuse it.

This is stronger than a privacy policy.

It is a systems constraint.

---

## Article IV — Recipient scope is explicit

A dead drop is not a broadcast.

A message is for the recipient or audience deliberately selected by the sender.

No component may silently expand that audience.

Forwarding, replication, publication, indexing, archival, model access, agent access, and third-party access are separate decisions.

The system must preserve that distinction.

Hidden audience expansion is an architectural failure.

---

## Article V — Local state belongs to the operator

A Deaddrop node belongs to its operator.

Local state is not a cache belonging to a platform.

The local system must remain conceptually primary even when network services are useful.

The network extends the node.

It does not become the owner of the node.

Where practical, a node should retain useful local behavior when disconnected from centralized infrastructure.

---

## Article VI — Share messages, not sandboxes

> **Share messages, not sandboxes.**

Private working state should remain private working state.

Agents, peers, machines, and runtimes should coordinate through explicit interfaces:

- messages
- immutable artifact references
- claims
- acknowledgments
- checkpoints
- provenance
- capability declarations
- authorized requests

They should not require unrestricted access to each other's mutable working environments.

A sandbox is a workshop.

It is not a coordination protocol.

---

## Article VII — Agents meet by reference, not by cohabitation

> **Agents should meet by reference, not by cohabitation.**

Distributed cooperation should not depend on pretending that many agents are one process.

An agent should be able to hand off work without exposing its full memory, scratch state, filesystem, process space, hidden prompts, credentials, or unrelated context.

References and provenance should make cooperation inspectable.

Isolation should remain meaningful.

---

## Article VIII — Authority is explicit

A model output is not automatically authority.

A signed message is not automatically authority.

A cache is not automatically authority.

A projection is not automatically authority.

A memory is not automatically authority.

A coordination log is not automatically authority.

Each domain must define where truth is owned.

Examples:

- a repository may own source code state
- a database may own application state
- a document may own a policy text
- an external service may own an account or transaction
- a human may own authorization for a consequential action

Deaddrop must preserve the difference between:

```text
LOCAL SANDBOX
ephemeral private work

DURABLE MEMORY
long-lived retained knowledge

COORDINATION HISTORY
messages, claims, ACKs, handoffs, checkpoints

SOURCE AUTHORITY
the system that actually owns the fact
```

Convenience must not collapse these categories.

---

## Article IX — Provenance travels with information

Information should retain enough provenance to answer:

- where did this come from?
- who or what produced it?
- what source does it refer to?
- what artifact or version was used?
- has the content changed?
- is this observation, inference, memory, or authority?

A result without provenance must be distinguishable from one with provenance.

Provenance does not make a claim true.

It makes the claim inspectable.

---

## Article X — Immutable reference beats ambiguous location

Mutable filenames, URLs, UI positions, and human labels are useful but insufficient as stable evidence.

Where integrity matters, artifacts should be referenceable by content identity.

For example:

```text
sha256:<digest>
```

The transport may change.

The storage location may change.

The descriptive name may change.

The identity of the referenced bytes must not.

---

## Article XI — Human authority remains legible

Automation should reduce mechanical work without making authority disappear.

Agents may search, compare, summarize, prepare, route, verify, and coordinate.

Consequential mutations should cross an explicit authorization boundary.

Examples include:

- purchases
- publication
- destructive operations
- account changes
- external commitments
- privilege changes
- actions with legal, financial, safety, or irreversible consequences

The human does not need to manually perform every step.

But the system must make clear when an agent is observing, proposing, preparing, or acting with delegated authority.

---

## Article XII — Capability graph, not popularity graph

Deaddrop should route based on relevance, authority, capability, trust, context, and provenance.

It should not recreate the incentive structure of social platforms.

The network should not optimize for:

- follower count
- virality
- engagement
- time on platform
- outrage
- trending status
- behavioral retention

The useful questions are:

```text
Who can do this?
Who is trusted for this domain?
Which source is authoritative?
Which artifact supports this claim?
Which peer can accept this handoff?
```

The system should optimize for successful work.

Not attention capture.

---

## Article XIII — Surveillance is not a business model

Deaddrop must not require behavioral surveillance in order to function as a product.

Core functionality should not depend on:

- cross-context behavioral profiling
- advertising identifiers
- hidden recommendation telemetry
- attention tracking
- engagement scoring
- opaque social graphs
- silent audience expansion
- data collection whose primary purpose is future monetization

Operational observability may be necessary to run reliable systems.

When it is, it must be:

- purpose-limited
- minimized
- documented
- separable from behavioral profiling
- retained only as long as justified

Reliability telemetry must not quietly become surveillance infrastructure.

---

## Article XIV — Minimize trust, not just document it

The system should assume that:

- intermediaries may be curious
- metadata can reveal information
- endpoints can be compromised
- agents can make mistakes
- peers can be malicious
- dependencies can fail
- operators can misconfigure systems

Security should therefore be designed around constrained knowledge, least privilege, explicit authority, verification, and isolation.

A component should not be trusted with information merely because it is operated by us.

---

## Article XV — Security-critical foundations are public

The public protocol must be inspectable.

Cryptographic choices should not depend on secrecy.

Message formats, authorization boundaries, integrity rules, protocol invariants, threat assumptions, and interoperability behavior should be reviewable.

Deaddrop should welcome hostile review of the public substrate.

Security claims become stronger when outsiders can attempt to break them.

---

## Article XVI — Defensive operations remain need-to-know

> **Open the protocol. Keep defensive operations need-to-know.**

Publishing the security model does not require publishing every operational defense.

The public repository may describe:

- threat classes
- guarantees
- trust assumptions
- protocol constraints
- abuse boundaries
- security goals

It does not need to expose exact production details such as:

- detection heuristics
- thresholds
- quarantine triggers
- anti-abuse signatures
- anomaly scoring
- defensive routing behavior
- incident-response playbooks
- decoy mechanisms
- operational countermeasures whose disclosure materially weakens them

This is not a substitute for sound protocol design.

It is operational security layered on top of an inspectable protocol.

---

## Article XVII — No hidden centrality

A relay is a transport mechanism.

It must not quietly become:

- the owner of identity
- the owner of the social graph
- the source of truth for private state
- a mandatory content index
- an advertising platform
- an engagement ranking system
- an irreversible dependency for local ownership

Central services may exist where useful.

They must not silently redefine the project around themselves.

---

## Article XVIII — No engagement traps

Deaddrop is an instrument.

You open it because you intend to do something.

Then you should be able to leave.

A feature should be treated with suspicion if its primary success metric is that the user remains inside the product longer than necessary.

Time-on-app is not a proxy for value.

The system should optimize for:

- task completion
- clarity
- verifiability
- safety
- low friction
- successful handoff
- correct authorization

Not captivity.

---

## Article XIX — Privacy is architectural, not cosmetic

A privacy settings page cannot compensate for architecture that exposes too much by default.

Privacy must influence:

- storage
- transport
- identity
- discovery
- indexing
- logging
- authorization
- agent access
- artifact handling
- defaults
- retention
- failure behavior

The safest option should not require expert configuration.

---

## Article XX — Conservative failure is acceptable

When privacy, authority, identity, provenance, or recipient scope is ambiguous, the system may refuse, defer, or ask for clarification.

It is acceptable for Deaddrop to fail closed.

It is not acceptable to silently widen access, invent authority, discard provenance, or infer consent for convenience.

---

## Constitutional test for changes

Any material change to Deaddrop should be reviewable against the following questions:

1. Does this expose information to a party that did not previously need it?
2. Does this widen an audience without an explicit decision?
3. Does this turn existence into discoverability or indexing?
4. Does this create new behavioral telemetry?
5. Does this collapse sandbox, memory, coordination history, and source authority?
6. Does this allow an intermediary to learn more than required?
7. Does this make human authority less legible?
8. Does this introduce engagement incentives unrelated to task success?
9. Does this create hidden centrality?
10. Does this weaken provenance or artifact integrity?
11. Does this move a defensive operational mechanism into unnecessary public detail?
12. If the answer to any of the above is yes, is the tradeoff explicit, narrow, reviewable, and genuinely necessary?

If the tradeoff cannot be explained clearly, the change should not merge.

---

## Amendment rule

This constitution may evolve.

It must not drift casually.

A change to these principles should be treated as an architectural decision, not a routine implementation detail.

A constitutional amendment should:

1. be explicit
2. explain the problem that requires the change
3. describe the security and privacy consequences
4. identify what previous guarantee is being altered
5. receive deliberate human review
6. remain visible in project history

Implementation convenience alone is not sufficient reason to weaken a constitutional invariant.

---

## Final principle

Deaddrop exists to make the network serve intent without requiring the person to surrender ownership, privacy, authority, or attention in exchange.

If the system becomes more convenient by becoming more surveillant, more centralized, less inspectable, or less explicit about authority, that is not progress.

It is architectural regression.
