# Capability Routing

**Status:** Public architecture baseline

## 1. The problem

A network of agents and peers needs a way to answer:

> Who or what can help with this intent?

The social-web answer is often popularity.

Deaddrop rejects that as the default routing primitive.

## 2. Capability graph

A capability graph describes relevant ability and context.

Examples:

```text
peer A
  can: rust-review
  scope: project-X

peer B
  can: source-verification
  scope: legal-documents

service C
  can: currency-quote
  authority: external-provider
```

The exact schema is not yet frozen.

## 3. Capability is not authority

A peer may be capable of doing something without being authoritative about the result.

Example:

- a model can summarize a contract
- the contract document remains authoritative for its wording

## 4. Capability is not permission

A peer may advertise an ability without being authorized to use resources on your behalf.

Routing and authorization remain separate.

## 5. Capability is not trust

A peer can claim a capability.

Local trust policy decides whether that claim is accepted for a context.

## 6. Contextual routing

Useful routing inputs may include:

- requested capability
- project/context
- source authority
- local trust
- availability
- protocol compatibility
- privacy constraints
- operator policy

The public architecture does not define a universal hidden ranking score.

## 7. No popularity economy

Deaddrop should not create routing incentives based primarily on:

- followers
- likes
- engagement
- virality
- time spent
- global popularity

A globally popular peer may be irrelevant to a specific task.

## 8. Explainability

Where practical, the node should be able to explain why a peer or service was selected:

```text
selected peer B
because:
- requested capability matched
- trusted locally for this domain
- protocol compatible
- source scope allowed
```

## 9. Defense boundary

Operational abuse scoring, anomaly detection, quarantine, and other defensive routing controls are not part of the public capability-ranking contract.
