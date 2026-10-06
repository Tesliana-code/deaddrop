//! Recipient verification. Pure: no relay, storage, or clock.

use deaddrop_protocol::{Ed25519PublicKey, EnvelopeV0, NodeId, SignatureV0, encode_envelope_v0};
use ed25519_dalek::{Signature, VerifyingKey};

/// Why a node refused a message it found addressed to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    NotAddressedToUs,
    /// `from` is not a peer this node has explicitly trusted.
    UntrustedSender,
    /// No record names `from` as signer with the trusted key for `from`.
    NoTrustedSignature,
    /// A record with the trusted key exists but does not verify.
    InvalidSignature,
    /// An acknowledgment that does not answer a message this node sent to
    /// the acknowledging peer.
    UnmatchedAcknowledgment,
}

impl Rejection {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotAddressedToUs => "not_addressed_to_us",
            Self::UntrustedSender => "untrusted_sender",
            Self::NoTrustedSignature => "no_trusted_signature",
            Self::InvalidSignature => "invalid_signature",
            Self::UnmatchedAcknowledgment => "unmatched_acknowledgment",
        }
    }
}

/// Accept `envelope` only if it is addressed to `local` and some record in
/// `signatures` with `signer == envelope.from` and `key == trusted` verifies
/// (strictly) over the exact canonical EnvelopeV0 bytes. Records with any
/// other signer, key, or message id are ignored: they can neither block nor
/// authorize acceptance.
pub fn verify(
    envelope: &EnvelopeV0,
    signatures: &[SignatureV0],
    trusted: Option<&Ed25519PublicKey>,
    local: &NodeId,
) -> Result<(), Rejection> {
    if envelope.to() != local {
        return Err(Rejection::NotAddressedToUs);
    }
    let trusted = trusted.ok_or(Rejection::UntrustedSender)?;
    let candidates: Vec<&SignatureV0> = signatures
        .iter()
        .filter(|record| {
            record.message_id() == envelope.id()
                && record.signer() == envelope.from()
                && record.key() == trusted
        })
        .collect();
    if candidates.is_empty() {
        return Err(Rejection::NoTrustedSignature);
    }
    let key =
        VerifyingKey::from_bytes(trusted.as_bytes()).map_err(|_| Rejection::InvalidSignature)?;
    let bytes = encode_envelope_v0(envelope).map_err(|_| Rejection::InvalidSignature)?;
    if candidates.iter().any(|record| {
        key.verify_strict(&bytes, &Signature::from_bytes(record.signature()))
            .is_ok()
    }) {
        Ok(())
    } else {
        Err(Rejection::InvalidSignature)
    }
}
