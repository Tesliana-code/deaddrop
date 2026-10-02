//! Recipient verification rules (pure; no relay, no storage).
//!
//! A message is accepted only when a signature record whose signer is
//! `envelope.from` and whose key is the locally trusted key for that peer
//! verifies over the exact canonical EnvelopeV0 bytes, and the envelope is
//! addressed to the local node. Other records never help or hurt.

use std::str::FromStr;

use deaddrop_protocol::{
    ArtifactRef, CorrelationId, EnvelopeV0, MessageId, MessageKind, NodeId, SignatureV0,
};
use deaddrop_shell::{LocalKey, Rejection, verify};

const A: &str = "node-a:shell:deaddrop";
const B: &str = "node-b:shell:deaddrop";
const C: &str = "node-c:shell:deaddrop";

fn node(value: &str) -> NodeId {
    NodeId::parse(value).unwrap()
}

fn key(seed: u8) -> LocalKey {
    LocalKey::from_seed([seed; 32])
}

fn message() -> EnvelopeV0 {
    EnvelopeV0::new(
        MessageId::parse("msg-1").unwrap(),
        node(A),
        node(B),
        MessageKind::Message,
        "hello",
    )
    .with_correlation_id(CorrelationId::parse("corr-1").unwrap())
    .with_artifact_ref(
        ArtifactRef::from_str(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap(),
    )
}

fn signed_by_a(envelope: &EnvelopeV0) -> SignatureV0 {
    key(1).sign(&node(A), envelope)
}

fn check(envelope: &EnvelopeV0, signatures: &[SignatureV0]) -> Result<(), Rejection> {
    verify(envelope, signatures, Some(&key(1).public()), &node(B))
}

#[test]
fn valid_trusted_signature_is_accepted() {
    let envelope = message();
    assert_eq!(check(&envelope, &[signed_by_a(&envelope)]), Ok(()));
}

#[test]
fn signature_covers_every_envelope_field() {
    let original = message();
    let sig = signed_by_a(&original);
    let tampered = [
        EnvelopeV0::new(
            original.id().clone(),
            node(A),
            node(B),
            MessageKind::Message,
            "hello!",
        )
        .with_correlation_id(CorrelationId::parse("corr-1").unwrap())
        .with_artifact_ref(original.artifact_refs()[0].clone()),
        EnvelopeV0::new(
            original.id().clone(),
            node(A),
            node(B),
            MessageKind::Handoff,
            "hello",
        )
        .with_correlation_id(CorrelationId::parse("corr-1").unwrap())
        .with_artifact_ref(original.artifact_refs()[0].clone()),
        EnvelopeV0::new(
            original.id().clone(),
            node(A),
            node(B),
            MessageKind::Message,
            "hello",
        )
        .with_correlation_id(CorrelationId::parse("corr-2").unwrap())
        .with_artifact_ref(original.artifact_refs()[0].clone()),
        EnvelopeV0::new(
            original.id().clone(),
            node(A),
            node(B),
            MessageKind::Message,
            "hello",
        )
        .with_correlation_id(CorrelationId::parse("corr-1").unwrap()),
    ];
    for envelope in tampered {
        assert_eq!(
            check(&envelope, std::slice::from_ref(&sig)),
            Err(Rejection::InvalidSignature),
            "{envelope:?}"
        );
    }
}

#[test]
fn unknown_sender_is_untrusted() {
    let envelope = message();
    assert_eq!(
        verify(&envelope, &[signed_by_a(&envelope)], None, &node(B)),
        Err(Rejection::UntrustedSender)
    );
}

#[test]
fn signature_by_another_key_is_not_trusted() {
    let envelope = message();
    // Same claimed signer, different key: e.g. C forging A.
    let forged = key(3).sign(&node(A), &envelope);
    assert_eq!(
        check(&envelope, &[forged]),
        Err(Rejection::NoTrustedSignature)
    );
}

#[test]
fn signer_must_equal_envelope_sender() {
    let envelope = message();
    // A's key, but the record names C as signer.
    let misattributed = key(1).sign(&node(C), &envelope);
    assert_eq!(
        check(&envelope, &[misattributed]),
        Err(Rejection::NoTrustedSignature)
    );
}

#[test]
fn record_for_another_message_does_not_count() {
    let envelope = message();
    let other = EnvelopeV0::new(
        MessageId::parse("msg-2").unwrap(),
        node(A),
        node(B),
        MessageKind::Message,
        "hello",
    );
    assert_eq!(
        check(&envelope, &[signed_by_a(&other)]),
        Err(Rejection::NoTrustedSignature)
    );
}

#[test]
fn unsigned_message_is_rejected() {
    assert_eq!(check(&message(), &[]), Err(Rejection::NoTrustedSignature));
}

#[test]
fn corrupt_signature_with_trusted_key_is_invalid() {
    let envelope = message();
    let good = signed_by_a(&envelope);
    let mut bytes = *good.signature();
    bytes[0] ^= 0xff;
    let corrupt = SignatureV0::new(
        good.message_id().clone(),
        good.signer().clone(),
        *good.key(),
        bytes,
    );
    assert_eq!(
        check(&envelope, &[corrupt]),
        Err(Rejection::InvalidSignature)
    );
}

#[test]
fn extra_records_neither_block_nor_authorize() {
    let envelope = message();
    let good = signed_by_a(&envelope);
    let noise = [
        key(3).sign(&node(A), &envelope),
        key(1).sign(&node(C), &envelope),
        key(3).sign(&node(C), &envelope),
    ];
    let mut with_good = noise.to_vec();
    with_good.push(good);
    assert_eq!(check(&envelope, &with_good), Ok(()));
    assert_eq!(check(&envelope, &noise), Err(Rejection::NoTrustedSignature));
}

#[test]
fn message_not_addressed_to_local_node_is_rejected() {
    let envelope = message();
    assert_eq!(
        verify(
            &envelope,
            &[signed_by_a(&envelope)],
            Some(&key(1).public()),
            &node(C)
        ),
        Err(Rejection::NotAddressedToUs)
    );
}
