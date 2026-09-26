# Threat Model

**Status:** Public baseline  
**Scope:** Classes of threats and trust assumptions. Operational detection and response details are intentionally out of scope.

## 1. Security objective

Deaddrop aims to let participants exchange messages, evidence, artifacts, and authorized actions while minimizing unnecessary disclosure and preserving provenance, integrity, recipient scope, and human authority.

The system does not assume that any intermediary is entitled to plaintext or unrelated private state.

## 2. Protected assets

Important assets include:

- message plaintext
- local private state
- signing and encryption keys
- peer relationships
- recipient scope
- artifact bytes and hashes
- provenance
- authorization state
- capability grants
- source credentials
- task and coordination history
- operator identity where applicable

## 3. Adversary classes

### Curious intermediary

A relay, host, storage provider, or network intermediary that performs its nominal role but attempts to learn more than required.

### Malicious peer

A peer that sends deceptive, malformed, replayed, abusive, or unauthorized messages.

### Compromised endpoint

A node whose local runtime, credentials, or filesystem has been compromised.

### Malicious or buggy agent

An agent that exceeds its task, misuses capabilities, fabricates claims, ignores recipient scope, or confuses inference with authority.

### Identity impersonator

An actor attempting to act as a trusted node, peer, or agent.

### Artifact substitution attacker

An actor attempting to replace referenced content while preserving a familiar filename, URL, label, or UI representation.

### Metadata observer

An actor attempting to infer relationships, timing, behavior, or activity from metadata even without plaintext.

### Dependency attacker

A compromised library, build dependency, update channel, integration, or service used by Deaddrop.

## 4. Threats in scope

The public architecture must address or explicitly bound:

- message tampering
- replay
- impersonation
- unauthorized capability use
- artifact substitution
- recipient-scope violations
- accidental audience expansion
- privilege confusion
- provenance loss
- source-authority confusion
- excessive intermediary knowledge
- metadata leakage
- insecure defaults
- malicious input
- duplicate delivery
- stale authorization
- key compromise and revocation
- untrusted external content
- compromised dependencies

## 5. Threats not solved by protocol alone

Some threats require endpoint and operational controls:

- a fully compromised operator device
- an authorized recipient intentionally leaking plaintext
- coercion of a participant
- malicious source systems returning false data
- social engineering outside the protocol
- physical compromise of unlocked devices

The protocol should limit blast radius where possible, but must not claim guarantees it cannot provide.

## 6. Security assumptions

Deaddrop should minimize assumptions, but some remain unavoidable:

- endpoints must protect local key material
- cryptographic primitives must be implemented correctly
- operators must be able to identify intended peers through a trustworthy enrollment path
- source systems remain authoritative only within the scope they actually own
- human authorization is meaningful only if the human-facing boundary is not itself compromised

## 7. Metadata

Encryption of content does not eliminate metadata.

The architecture must treat metadata as potentially sensitive and minimize collection and retention where practical.

The public threat model may describe metadata classes.

Operational techniques used to detect attacks from metadata belong to the private defensive layer when disclosure would weaken them.

## 8. Agent-specific risks

Agents introduce risks beyond ordinary messaging:

- prompt or content injection
- false authority attribution
- capability overreach
- unintended disclosure through tool use
- acting on stale or unverified state
- chain-of-agent provenance loss
- automation amplifying a small mistake

Agent outputs must remain distinguishable from source authority.

Consequential action must cross explicit authorization policy.

## 9. Defensive boundary

This document intentionally does not publish:

- detection heuristics
- abuse thresholds
- quarantine triggers
- anomaly scores
- signatures
- decoys
- incident playbooks
- defensive routing tactics
- live countermeasure configuration

Those are operational defenses, not protocol invariants.

## 10. Security review question

For every meaningful change:

> What new information, authority, trust, or attack surface does this introduce, and which component now knows or can do something it could not before?

If the answer is unclear, the change is not ready.
