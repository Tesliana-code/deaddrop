# ADR-002: Ed25519 detached signatures for V0 messages

**Status:** Accepted  
**Date:** 2026-10-02

## Context

The network shell needs two independent nodes to exchange messages through
an asynchronous mailbox relay, with the recipient able to prove which peer
sent each message. `IDENTITY_AND_TRUST.md` requires that a node prove control
of a cryptographic identity, that algorithms be frozen by ADR, and that only
established, reviewable libraries be used.

`EnvelopeV0` and its canonical encoding are frozen, with published vectors
and downstream consumers. It has no signature field.

## Decision

1. **Suite.** Ed25519 (RFC 8032), implemented by `ed25519-dalek`, with
   verification by `verify_strict`. Secret keys come from the operating
   system CSPRNG (`getrandom`) and are stored only in the node's home
   directory, owner-readable (0600).
2. **What is signed.** The exact canonical `encode_envelope_v0` bytes. Every
   envelope field (id, from, to, kind, correlation, body, artifact refs) is
   therefore covered.
3. **Carriage.** A detached record, `SignatureV0` (`deaddrop-sig/0`):
   `{protocol, message_id, signer, key: "ed25519:<hex>", signature: "<hex>"}`
   with one canonical encoding. `EnvelopeV0` is unchanged; the envelope
   protocol is not bumped; existing vectors are untouched.
4. **Relay role.** The node stores and returns signature records through
   additive routes (`POST`/`GET /v0/messages/{id}/signatures`). It never
   verifies and never chooses among records. Distinct records for one
   message id accumulate.
5. **Acceptance rule.** A recipient accepts a message only when:
   - `envelope.to` is the local node;
   - `envelope.from` is an explicitly trusted peer;
   - a record with `signer == envelope.from` and `key ==` the trusted key for
     that peer verifies over the canonical envelope bytes.

   Any other record (other signer, other key, other message id, or a
   failing signature) neither blocks nor authorizes acceptance.
6. **Crypto placement.** `deaddrop-protocol` defines only the record's data
   shape and text formats. Signing and verification live in
   `deaddrop-shell`.

## Alternatives considered

- **Envelope version bump with an embedded signature.** Breaks the frozen
  vectors and every pinned consumer for a property that can travel beside
  the envelope.
- **Wrapping the envelope in a signed carrier body.** Duplicates addressing
  and makes the relay-visible envelope disagree with the signed one.
- **Signatures as artifacts.** An envelope cannot reference its own
  signature, and the relay has no index from message to artifact.

## Consequences

- Recipients can authenticate senders without trusting the relay.
- Key rotation, revocation, and multi-key peers are not yet specified; a
  peer's key cannot change silently (re-adding with a different key fails).
- The relay can be flooded with junk signature records for a message id;
  they cannot affect acceptance, but storage is unbounded in V0.

## Encryption boundary

Slice 1 message bodies are **signed but not encrypted**. The relay can read
them. This is acceptable only for the local loopback development relay, and
`deaddrop init` refuses any non-loopback relay.

**Encryption is required before a non-loopback relay is treated as
production-capable.** Slice 1 must not be presented as an encrypted network
shell.

## Constitutional impact

Strengthens provenance and recipient scope; keeps the relay a mailbox rather
than an identity authority; adds no global identity registry or peer graph.
