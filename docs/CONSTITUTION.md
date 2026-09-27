# Deaddrop Constitution

**Status:** Foundational architectural invariant  
**Applies to:** protocol, clients, relays, services, storage, agents, integrations, tooling, and governance

Deaddrop is defined by architectural constraints that remain stable across UI, transport, database, model, runtime, and implementation language.

Implementations may evolve.

These principles preserve the identity of the system through that evolution.

A contribution belongs in Deaddrop when it strengthens these principles in practice.

---

## Article I — Publicness is explicit

> **Publicness is explicit.**

Creation, persistence, indexing, publication, redistribution, training access, and audience expansion are distinct actions.

Each action carries its own scope and authority.

Deaddrop components preserve those distinctions.

Public intent is represented explicitly.

---

## Article II — Indexing requires scope and authority

> **Indexing requires explicit scope and authority.**

Indexing creates discoverability and changes the audience and risk profile of information.

Every index therefore has:

- a defined scope
- a defined authority
- a defined purpose
- a defined retention model
- a defined audience

Ambiguity resolves toward the narrower scope.

---

## Article III — Infrastructure minimizes knowledge

> **Infrastructure learns the minimum required for its role.**

Intermediaries receive the smallest useful view of the system.

A relay routes encrypted payloads.

An artifact store preserves bytes and integrity.

Discovery operates within explicit scope.

Routing receives the context required for routing.

Services receive purpose-limited information.

The architecture reduces intermediary knowledge through system design.

This is a systems constraint.

---

## Article IV — Recipient scope is preserved

A dead drop is recipient-scoped exchange.

The sender selects the recipient or audience.

The system preserves that scope across transport, storage, replication, archival, indexing, model access, agent access, and third-party integration.

Audience changes are explicit events with explicit authority.

Recipient scope remains inspectable end to end.

---

## Article V — Local state belongs to the operator

A Deaddrop node belongs to its operator.

Local state is primary operator-owned state.

Network services extend local capability.

Disconnected operation remains useful wherever practical.

Local ownership stays meaningful across deployment models.

---

## Article VI — Coordination happens through explicit exchange

> **Share messages. Keep sandboxes private.**

Private working state remains private working state.

Agents, peers, machines, and runtimes coordinate through explicit interfaces:

- messages
- immutable artifact references
- claims
- acknowledgments
- checkpoints
- provenance
- capability declarations
- authorized requests

A sandbox is a workshop.

The coordination protocol is the explicit exchange between workshops.

---

## Article VII — Agents meet by reference

> **Agents meet by reference across isolated runtimes.**

Distributed cooperation uses explicit handoffs between independent agents.

An agent shares the information required for the handoff while retaining its private memory, scratch state, filesystem, process space, prompts, credentials, and unrelated context.

References and provenance make cooperation inspectable.

Isolation remains a first-class architectural property.

---

## Article VIII — Authority is explicit

Each domain defines where truth is owned.

Examples:

- a repository may own source code state
- a database may own application state
- a document may own policy text
- an external service may own an account or transaction
- a human may own authorization for a consequential action

Deaddrop preserves the distinction between:

```text
LOCAL SANDBOX
ephemeral private work

DURABLE MEMORY
long-lived retained knowledge

COORDINATION HISTORY
messages, claims, ACKs, handoffs, checkpoints

SOURCE AUTHORITY
the system that owns the fact
```

Model output is interpreted as model output.

Signed messages are interpreted as signed messages.

Caches are interpreted as caches.

Projections are interpreted as derived views.

Memory is interpreted as retained knowledge.

Coordination logs are interpreted as coordination history.

Authority is named by the domain that owns it.

---

## Article IX — Provenance travels with information

Information retains enough provenance to answer:

- where did this come from?
- who or what produced it?
- what source does it refer to?
- what artifact or version was used?
- has the content changed?
- is this observation, inference, memory, or authority?

Provenance makes a claim inspectable.

Confidence and authority remain separate dimensions.

---

## Article X — Content identity anchors artifact integrity

Mutable filenames, URLs, UI positions, and human labels provide useful navigation.

Content identity provides stable evidence.

Where integrity matters, artifacts are referenceable by content identity.

For example:

```text
sha256:<digest>
```

Transport, storage location, and descriptive names may evolve around a stable byte identity.

---

## Article XI — Human authority remains legible

Automation reduces mechanical work while preserving visible authority boundaries.

Agents may search, compare, summarize, prepare, route, verify, and coordinate.

Consequential mutations cross an explicit authorization boundary.

Examples include:

- purchases
- publication
- destructive operations
- account changes
- external commitments
- privilege changes
- actions with legal, financial, safety, or irreversible consequences

The system makes clear when an agent is observing, proposing, preparing, or acting with delegated authority.

---

## Article XII — Capability routing drives discovery

Deaddrop routes based on relevance, authority, capability, trust, context, and provenance.

The useful questions are:

```text
Who can do this?
Who is trusted for this domain?
Which source is authoritative?
Which artifact supports this claim?
Which peer can accept this handoff?
```

The system optimizes for successful work, correct routing, and verifiable handoff.

---

## Article XIII — Privacy shapes product operation

Core functionality works through recipient scope, purpose limitation, least privilege, and explicit authority.

Operational observability supports reliability through:

- purpose-limited collection
- data minimization
- documented semantics
- separation from behavioral profiling
- justified retention
- explicit access boundaries

Product value comes from successful work, reliability, and trust.

---

## Article XIV — Trust is minimized through architecture

The system plans for curious intermediaries, revealing metadata, compromised endpoints, agent error, malicious peers, dependency failure, and operator misconfiguration.

Security therefore uses:

- constrained knowledge
- least privilege
- explicit authority
- verification
- isolation
- auditable boundaries

Every component receives the minimum trust and information required for its function.

---

## Article XV — Security-critical foundations are inspectable

The public protocol is inspectable.

Cryptographic choices are reviewable.

Message formats, authorization boundaries, integrity rules, protocol invariants, threat assumptions, and interoperability behavior are documented.

Deaddrop welcomes adversarial review of the public substrate.

Security claims strengthen through independent scrutiny.

---

## Article XVI — Defensive operations follow need-to-know boundaries

> **Open the protocol. Keep defensive operations need-to-know.**

The public repository documents:

- threat classes
- guarantees
- trust assumptions
- protocol constraints
- abuse boundaries
- security goals

Production defense retains deployment-specific details within operational scope, including:

- detection heuristics
- thresholds
- quarantine triggers
- anti-abuse signatures
- anomaly scoring
- defensive routing behavior
- incident-response playbooks
- decoy mechanisms
- live countermeasures

Inspectable protocol design provides the foundation.

Operational security adds deployment-specific protection above it.

---

## Article XVII — Services remain subordinate to local ownership

Relays and central services provide bounded capabilities.

A relay transports messages.

Identity authority remains explicit.

Private-state authority remains explicit.

Indexes remain scoped.

Local ownership remains portable.

Service boundaries remain visible and replaceable.

---

## Article XVIII — The product optimizes for completed work

Deaddrop is an instrument.

You open it because you intend to do something.

You leave when the task is complete.

Success metrics center on:

- task completion
- clarity
- verifiability
- safety
- low friction
- successful handoff
- correct authorization
- reliability

Attention remains available to the human for everything beyond the tool.

---

## Article XIX — Privacy is architectural

Privacy shapes:

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

Safe defaults provide strong protection from the first run.

Expert configuration can refine policy while preserving the architectural baseline.

---

## Article XX — Ambiguity resolves conservatively

Privacy, authority, identity, provenance, and recipient scope use explicit evidence.

Ambiguous cases resolve through one of three bounded outcomes:

- refuse the operation
- defer the operation
- request clarification

The system preserves the narrowest proven scope and the strongest known provenance until authority becomes clear.

Fail-closed behavior is a valid and intentional outcome.

---

## Constitutional test for changes

Every material change to Deaddrop should answer these questions clearly:

1. Is the audience explicit?
2. Is indexing explicitly scoped and authorized?
3. Is behavioral telemetry purpose-limited and justified?
4. Are sandbox, memory, coordination history, and source authority still distinct?
5. Does every intermediary receive the minimum required information?
6. Is human authority legible?
7. Are success metrics aligned with task completion?
8. Are central services bounded and replaceable?
9. Is provenance preserved?
10. Is artifact integrity preserved?
11. Are defensive operational details kept within their proper scope?
12. Are every tradeoff and authority boundary explicit, narrow, and reviewable?

A change is merge-ready when these answers are clear and consistent with the constitution.

---

## Amendment rule

This constitution evolves through deliberate architectural decisions.

A constitutional amendment should:

1. be explicit
2. explain the problem that requires the change
3. describe the security and privacy consequences
4. identify the guarantee being altered
5. receive deliberate human review
6. remain visible in project history

Implementation convenience is evaluated alongside the constitutional guarantees it affects.

---

## Final principle

Deaddrop exists to make the network serve human intent while preserving ownership, privacy, authority, and attention.

Progress strengthens local ownership, inspectability, explicit authority, provenance, privacy, and bounded infrastructure.

That is the architecture.
