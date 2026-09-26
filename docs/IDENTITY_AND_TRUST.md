# Identity and Trust

**Status:** Public architecture baseline

## 1. Identity is not one thing

Deaddrop distinguishes:

- human/operator identity
- device/node identity
- agent/runtime identity
- cryptographic identity
- external service identity

These may be related but must not be silently collapsed.

## 2. Logical node identity

The working human-readable shape is:

```text
host:harness:project
```

Example:

```text
workstation-7:local-agent:deaddrop
```

This is a routing and context label.

It is not, by itself, cryptographic proof.

## 3. Cryptographic identity

A node must be able to prove control of a cryptographic identity used to authenticate messages.

Specific algorithms and key formats will be frozen by ADR.

The project will use established, reviewable cryptographic libraries and will not invent custom primitives.

## 4. Enrollment

Trust establishment is a separate act from discovering that an identity exists.

A node may know about a peer without trusting it.

A node may trust a peer for one context without granting broad capabilities.

Enrollment must have a verifiable path appropriate to the deployment.

## 5. Trust is contextual

Trust should answer questions such as:

- trust this key to represent this peer?
- trust this peer to provide information in this domain?
- trust this peer to receive this class of message?
- trust this peer with this capability?

A single universal trust score is deliberately avoided.

## 6. Identity, trust, reputation, and authorization differ

```text
IDENTITY
who is this?

TRUST
what claims or behavior do I accept from this peer in this context?

REPUTATION
what historical evidence exists about behavior?

AUTHORIZATION
what may this actor actually do?
```

One must not silently imply another.

## 7. Rotation and revocation

Keys and devices will eventually change.

The architecture must support:

- key rotation
- revoked identities
- replaced devices
- stale credentials
- historical verification

Revocation semantics must be explicit and versioned.

## 8. Privacy

Identity systems should avoid requiring a global public social graph.

Peer relationships are local state unless deliberately shared.

## 9. Defense boundary

Public docs specify identity proofs and trust semantics.

Private defense may apply additional abuse detection, quarantine, or risk controls without turning those mechanisms into protocol requirements.
