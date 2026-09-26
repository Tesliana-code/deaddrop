# Messaging

**Status:** Public architecture baseline

## 1. A message is a dead drop, not a post

The default Deaddrop message is addressed communication.

It has an intended recipient or audience.

It is not public merely because infrastructure can transport it.

## 2. Core flow

```text
compose
  ↓
bind recipient
  ↓
bind provenance / artifact refs
  ↓
sign
  ↓
protect content as required
  ↓
transport
  ↓
receive
  ↓
verify
  ↓
persist according to local policy
  ↓
ACK
```

## 3. Asynchronous by default

Peers need not be online simultaneously.

Messages should survive:

- temporary disconnection
- process restart
- transport retry
- delayed delivery

This makes a relay useful as a mailbox without making it a platform.

## 4. Message identity

Retransmission does not create a new semantic message.

Receivers need enough identity to recognize duplicates and apply idempotent behavior.

## 5. Correlation

A multi-step interaction should be traceable without requiring shared memory.

Correlation identifiers can connect:

- request and response
- delegation and completion
- task and checkpoints
- handoff and acknowledgment

Correlation is not authority.

## 6. Handoffs

A good handoff should carry only what the receiver needs:

- intent
- constraints
- relevant provenance
- artifact references
- expected output
- authority boundaries
- correlation identity

It should not require copying an agent's entire hidden context or sandbox.

## 7. Acknowledgments

Receipt, acceptance, completion, and external side-effect success are different states.

The protocol should name them distinctly rather than collapsing them into one generic ACK.

## 8. Ordering

Network arrival order must not be assumed to equal semantic order.

Where order matters, it must be expressed through protocol state, causality, revision, or explicit sequencing.

## 9. Privacy

Message metadata should be minimized.

The fact that a relay can observe delivery metadata does not grant permission to build behavioral profiles from it.

## 10. No feed semantics

Deaddrop messaging must not accidentally become a feed ranked by engagement.

If broadcast or group communication is introduced later, recipient scope and delivery semantics remain explicit.
